use anyhow::{Context as _, Result};
#[cfg(test)]
use crossbeam_channel::unbounded;
use crossbeam_channel::{Receiver, SendTimeoutError, Sender, bounded};
use eternalist_apps::NativeWake;
use roaring::RoaringBitmap;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, PoisonError},
    thread,
    time::{Duration, Instant},
};

use crate::{
    booru::{Booru as _, Danbooru, TagDefinition},
    config::MirrorPolicy,
    date::DateRange,
    index::{CacheStats, FactMergeBudget, Index, TagSuggestion},
    kin::Backfill,
    media::{MediaCache, RgbaBlade, required_url},
    model::{
        Corpus, FamilyTree, GalleryTopology, PostId, PostRecord, Query, SearchHit, Sort, Tag,
        TagPattern, TagPatternMatch,
    },
};

const DANBOORU_READ_GAP: Duration = Duration::from_millis(150);
const SUGGESTION_LIMIT: usize = 12;
const EVENT_CAPACITY: usize = 32;
const REFRESH_COMMAND_CAPACITY: usize = 64;
const WARM_COMMAND_CAPACITY: usize = 16;
const FAMILY_COMMAND_CAPACITY: usize = 4;
const DEFINITION_COMMAND_CAPACITY: usize = 16;
const MEDIA_COMMAND_CAPACITY: usize = 512;
const MEDIA_PENDING_CAPACITY: usize = 512;
const MEDIA_FETCHERS: usize = 3;
const MEDIA_OFFER_PATIENCE: Duration = Duration::from_millis(50);
const CRAWL_GAP: Duration = Duration::ZERO;
const CRAWL_EMPTY_GAP: Duration = Duration::from_mins(1);
const CRAWL_FAULT_GAP: Duration = Duration::from_secs(5);
const MERGE_GAP: Duration = Duration::from_millis(250);
const MERGE_IDLE_GAP: Duration = Duration::from_secs(2);
const MAX_FAMILY_FETCHES: usize = 256;
const FAMILY_POST_BATCH: usize = 100;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct BladeEpoch(u64);

impl BladeEpoch {
    pub const ROOT: Self = Self(0);

    pub fn advance(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

#[derive(Debug)]
pub enum Command {
    Warm {
        query: Query,
        sort: Sort,
        first_page: u32,
        pages: u32,
    },
    Refresh {
        serial: u64,
        query: Query,
        local_favorites: Arc<RoaringBitmap>,
        corpus: Corpus,
        sort: Sort,
        dates: DateRange,
        topology: GalleryTopology,
        limit: usize,
    },
    Stats {
        serial: u64,
    },
    Suggest {
        serial: u64,
        prefix: String,
    },
    ResolvePatterns {
        serial: u64,
        patterns: Vec<TagPattern>,
    },
    Family {
        serial: u64,
        id: PostId,
    },
    TagDefinition {
        serial: u64,
        tag: Tag,
    },
    Blade {
        epoch: BladeEpoch,
        id: PostId,
        bucket: u8,
        url: Option<String>,
    },
    CullBlades {
        epoch: BladeEpoch,
    },
    FullBlade {
        id: PostId,
        url: Option<String>,
        texture_bound: [u32; 2],
    },
    SaveMedia {
        id: PostId,
        url: Option<String>,
        path: PathBuf,
    },
    /// Re-absorb one post from upstream (heals records predating tag hints).
    Refetch {
        id: PostId,
    },
    /// Warm the disk media cache with a post's full image, fire-and-forget.
    Prefetch {
        id: PostId,
        url: Option<String>,
    },
}

#[derive(Debug)]
pub enum Event {
    Refreshed {
        serial: u64,
        hit: SearchHit,
    },
    RefreshFault {
        serial: u64,
        fault: String,
    },
    Stats {
        serial: u64,
        stats: CacheStats,
    },
    StatsFault {
        serial: u64,
        fault: String,
    },
    Suggested {
        serial: u64,
        hits: Vec<TagSuggestion>,
    },
    PatternsResolved {
        serial: u64,
        expansions: Vec<(TagPattern, Vec<TagPatternMatch>)>,
    },
    Warmed {
        query_key: String,
        sort: Sort,
        first_page: u32,
        pages: u32,
        posts: usize,
        exhausted: bool,
    },
    WarmFault {
        query_key: String,
        sort: Sort,
        first_page: u32,
        fault: String,
    },
    Crawled {
        posts: usize,
        before: Option<PostId>,
    },
    KinCrawled {
        posts: usize,
        before: Option<PostId>,
        complete: bool,
    },
    Family {
        serial: u64,
        tree: FamilyTree,
    },
    FamilyFault {
        serial: u64,
        fault: String,
    },
    TagDefinition {
        serial: u64,
        tag: Tag,
        result: std::result::Result<Option<TagDefinition>, String>,
    },
    TagDefinitionsCancelled(Vec<(u64, Tag)>),
    Refetched {
        post: Option<Box<PostRecord>>,
    },
    Blade {
        bucket: u8,
        blade: RgbaBlade,
    },
    BladeFault {
        id: PostId,
        bucket: u8,
        fault: String,
    },
    FullBlade {
        blade: RgbaBlade,
        display: Option<RgbaBlade>,
    },
    FullBladeFault {
        id: PostId,
        fault: String,
    },
    MediaSaved {
        id: PostId,
        path: PathBuf,
    },
    MediaSaveFault {
        id: PostId,
        fault: String,
    },
    FactsMerged {
        batches: usize,
        bytes: usize,
        groups: usize,
    },
    Toast(String),
    Fault(String),
}

/// Event sender that also wakes the UI event loop while the window is focused.
/// The bounded event channel becomes backpressure instead of a repaint clock
/// while the application is in the background.
#[derive(Clone)]
pub struct Klaxon {
    tx: Sender<Event>,
    wake: NativeWake,
}

/// How long status-grade events may pool before a repaint shows them.
const MURMUR: Duration = Duration::from_millis(350);

impl Klaxon {
    fn send(&self, event: Event) {
        let _sent = self.tx.send(event);
        let _woken = self.wake.request_foreground_repaint();
    }

    /// For events that only move status lines: foreground wakes coalesce, and
    /// a background crawl cannot force a full repaint per page.
    fn murmur(&self, event: Event) {
        let _sent = self.tx.send(event);
        let _woken = self.wake.request_foreground_repaint_after(MURMUR);
    }

    /// User-facing one-liner from any thread.
    pub fn toast(&self, text: String) {
        self.send(Event::Toast(text));
    }
}

pub struct Worker {
    refresh_tx: Sender<RefreshCommand>,
    warm_tx: Sender<WarmCommand>,
    family_tx: Sender<FamilyCommand>,
    definition_tx: Option<Sender<DefinitionCommand>>,
    media_tx: Sender<MediaCommand>,
    mirror: MirrorValve,
    crier: Klaxon,
    rx: Receiver<Event>,
}

impl Worker {
    pub fn spawn(
        index: Index,
        media: MediaCache,
        ctx: egui::Context,
        mirror_policy: MirrorPolicy,
    ) -> Self {
        let (refresh_tx, refresh_rx) = bounded(REFRESH_COMMAND_CAPACITY);
        let (warm_tx, warm_rx) = bounded(WARM_COMMAND_CAPACITY);
        let (family_tx, family_rx) = bounded(FAMILY_COMMAND_CAPACITY);
        let (media_tx, media_rx) = bounded(MEDIA_COMMAND_CAPACITY);
        let (event_tx, event_rx) = bounded(EVENT_CAPACITY);
        let event_tx = Klaxon {
            tx: event_tx,
            wake: NativeWake::from_context(&ctx),
        };
        let crier = event_tx.clone();
        let refresh_events = event_tx.clone();
        let refresh_index = index.clone();
        let _refresh =
            thread::spawn(move || refresh_loop(refresh_index, refresh_rx, refresh_events));
        let read_gate = RateGate::new(DANBOORU_READ_GAP);
        let booru = Danbooru::new();
        let warm_booru = booru.clone();
        let warm_events = event_tx.clone();
        let warm_index = index.clone();
        let warm_gate = read_gate.clone();
        let _warm = thread::spawn(move || {
            warm_loop(warm_booru, warm_index, warm_gate, warm_rx, warm_events);
        });
        let family_booru = booru.clone();
        let family_index = index.clone();
        let family_gate = read_gate.clone();
        let family_events = event_tx.clone();
        let _family = thread::spawn(move || {
            family_loop(
                family_booru,
                family_index,
                family_gate,
                family_rx,
                family_events,
            );
        });
        let definition_tx = booru.tag_definitions().map(|_| {
            let (definition_tx, definition_rx) = bounded(DEFINITION_COMMAND_CAPACITY);
            let definition_booru = booru.clone();
            let definition_gate = read_gate.clone();
            let definition_events = event_tx.clone();
            let _definitions = thread::spawn(move || {
                definition_loop(
                    definition_booru,
                    definition_gate,
                    definition_rx,
                    definition_events,
                );
            });
            definition_tx
        });
        // One dispatcher keeps epoch culling and full-blade priority coherent;
        // a small fetcher pool overlaps network latency so thumbnails land in
        // parallel instead of one per round trip.
        let (media_work_tx, media_work_rx) = bounded::<MediaCommand>(0);
        let _media_dispatch = thread::spawn(move || media_dispatch(media_rx, media_work_tx));
        for _ in 0..MEDIA_FETCHERS {
            let fetch_cache = media.clone();
            let fetch_events = event_tx.clone();
            let fetch_work = media_work_rx.clone();
            let _fetcher =
                thread::spawn(move || media_fetch_loop(fetch_cache, fetch_work, fetch_events));
        }
        let crawl_index = index.clone();
        let crawl_events = event_tx.clone();
        let crawl_gate = read_gate.clone();
        let mirror = MirrorValve::new(mirror_policy);
        let kin_booru = booru.clone();
        let kin_index = index.clone();
        let kin_gate = read_gate.clone();
        let kin_events = event_tx.clone();
        let kin_mirror = mirror.clone();
        let _kin = thread::spawn(move || {
            kin_loop(kin_booru, kin_index, kin_gate, kin_mirror, kin_events);
        });
        let crawl_mirror = mirror.clone();
        let _crawl = thread::spawn(move || {
            crawl_loop(booru, crawl_index, crawl_gate, crawl_mirror, crawl_events);
        });
        let merge_events = event_tx.clone();
        let _merge = thread::spawn(move || merge_loop(index, merge_events));
        Self {
            refresh_tx,
            warm_tx,
            family_tx,
            definition_tx,
            media_tx,
            mirror,
            crier,
            rx: event_rx,
        }
    }

    /// A handle for app-side threads (e.g. the clipboard) to report back.
    pub fn crier(&self) -> Klaxon {
        self.crier.clone()
    }

    pub fn send(&self, command: Command) -> Result<()> {
        match command {
            Command::Refresh {
                serial,
                query,
                local_favorites,
                corpus,
                sort,
                dates,
                topology,
                limit,
            } => self
                .refresh_tx
                .try_send(RefreshCommand::Search {
                    serial,
                    query,
                    local_favorites,
                    corpus,
                    sort,
                    dates,
                    topology,
                    limit,
                })
                .context("send refresh worker command"),
            Command::Stats { serial } => self
                .refresh_tx
                .try_send(RefreshCommand::Stats { serial })
                .context("send stats worker command"),
            Command::Suggest { serial, prefix } => self
                .refresh_tx
                .try_send(RefreshCommand::Suggest { serial, prefix })
                .context("send suggest worker command"),
            Command::ResolvePatterns { serial, patterns } => self
                .refresh_tx
                .try_send(RefreshCommand::ResolvePatterns { serial, patterns })
                .context("send tag-pattern worker command"),
            Command::Family { serial, id } => self
                .family_tx
                .try_send(FamilyCommand { serial, id })
                .context("send family worker command"),
            Command::TagDefinition { serial, tag } => self
                .definition_tx
                .as_ref()
                .context("provider has no tag-definition source")?
                .try_send(DefinitionCommand { serial, tag })
                .context("send tag-definition worker command"),
            Command::Warm {
                query,
                sort,
                first_page,
                pages,
            } => self
                .warm_tx
                .try_send(WarmCommand::Warm {
                    query,
                    sort,
                    first_page,
                    pages,
                })
                .context("send warm worker command"),
            Command::Blade {
                epoch,
                id,
                bucket,
                url,
            } => self
                .media_tx
                .try_send(MediaCommand::Blade {
                    epoch,
                    id,
                    bucket,
                    url,
                })
                .context("send media worker command"),
            Command::CullBlades { epoch } => self
                .media_tx
                .try_send(MediaCommand::Cull { epoch })
                .context("send media worker command"),
            Command::FullBlade {
                id,
                url,
                texture_bound,
            } => self
                .media_tx
                .try_send(MediaCommand::FullBlade {
                    id,
                    url,
                    texture_bound,
                })
                .context("send media worker command"),
            Command::SaveMedia { id, url, path } => self
                .media_tx
                .try_send(MediaCommand::Save { id, url, path })
                .context("send media worker command"),
            Command::Refetch { id } => self
                .warm_tx
                .try_send(WarmCommand::Refetch(id))
                .context("send refetch worker command"),
            Command::Prefetch { id, url } => self
                .media_tx
                .try_send(MediaCommand::Prefetch { id, url })
                .context("send prefetch worker command"),
        }
    }

    pub fn take_event(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }

    pub fn has_pending_events(&self) -> bool {
        !self.rx.is_empty()
    }

    pub fn has_tag_definitions(&self) -> bool {
        self.definition_tx.is_some()
    }

    pub fn set_mirror_policy(&self, policy: MirrorPolicy) {
        self.mirror.set(policy);
    }
}

#[derive(Clone)]
struct MirrorValve(Arc<(Mutex<MirrorPolicy>, Condvar)>);

impl MirrorValve {
    fn new(policy: MirrorPolicy) -> Self {
        Self(Arc::new((Mutex::new(policy), Condvar::new())))
    }

    fn set(&self, policy: MirrorPolicy) {
        let (state, changed) = &*self.0;
        *state.lock().unwrap_or_else(PoisonError::into_inner) = policy;
        changed.notify_all();
    }

    fn await_flow(&self) {
        let (state, changed) = &*self.0;
        let mut policy = state.lock().unwrap_or_else(PoisonError::into_inner);
        while !policy.active() {
            policy = changed.wait(policy).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

#[derive(Debug)]
enum RefreshCommand {
    Search {
        serial: u64,
        query: Query,
        local_favorites: Arc<RoaringBitmap>,
        corpus: Corpus,
        sort: Sort,
        dates: DateRange,
        topology: GalleryTopology,
        limit: usize,
    },
    Stats {
        serial: u64,
    },
    Suggest {
        serial: u64,
        prefix: String,
    },
    ResolvePatterns {
        serial: u64,
        patterns: Vec<TagPattern>,
    },
}

#[derive(Clone, Copy, Debug)]
struct FamilyCommand {
    serial: u64,
    id: PostId,
}

#[derive(Debug)]
struct DefinitionCommand {
    serial: u64,
    tag: Tag,
}

#[derive(Debug)]
enum WarmCommand {
    Warm {
        query: Query,
        sort: Sort,
        first_page: u32,
        pages: u32,
    },
    Refetch(PostId),
}

#[derive(Debug)]
enum MediaCommand {
    Blade {
        epoch: BladeEpoch,
        id: PostId,
        bucket: u8,
        url: Option<String>,
    },
    Cull {
        epoch: BladeEpoch,
    },
    FullBlade {
        id: PostId,
        url: Option<String>,
        texture_bound: [u32; 2],
    },
    Save {
        id: PostId,
        url: Option<String>,
        path: PathBuf,
    },
    Prefetch {
        id: PostId,
        url: Option<String>,
    },
}

fn refresh_loop(index: Index, commands: Receiver<RefreshCommand>, events: Klaxon) {
    while let Ok(first) = commands.recv() {
        let mut search = None;
        let mut stats = None;
        let mut suggest = None;
        let mut patterns = None;
        collect_refresh(first, &mut search, &mut stats, &mut suggest, &mut patterns);
        for command in commands.try_iter() {
            collect_refresh(
                command,
                &mut search,
                &mut stats,
                &mut suggest,
                &mut patterns,
            );
        }
        if let Some((serial, prefix)) = suggest {
            let event = match index.tag_suggestions(&prefix, SUGGESTION_LIMIT) {
                Ok(hits) => Event::Suggested { serial, hits },
                Err(err) => Event::Fault(format!("{err:#}")),
            };
            events.send(event);
        }
        if let Some((serial, patterns)) = patterns {
            let event = match index.tag_pattern_matches(&patterns) {
                Ok(expansions) => Event::PatternsResolved { serial, expansions },
                Err(err) => Event::Fault(format!("{err:#}")),
            };
            events.send(event);
        }
        if let Some((serial, query, local_favorites, corpus, sort, dates, topology, limit)) = search
        {
            let event = match index.search_corpus(
                &query,
                &local_favorites,
                corpus,
                sort,
                dates,
                topology,
                limit,
            ) {
                Ok(hit) => Event::Refreshed { serial, hit },
                Err(err) => Event::RefreshFault {
                    serial,
                    fault: format!("{err:#}"),
                },
            };
            events.send(event);
        }
        if let Some(serial) = stats {
            let event = match index.stats() {
                Ok(stats) => Event::Stats { serial, stats },
                Err(err) => Event::StatsFault {
                    serial,
                    fault: format!("{err:#}"),
                },
            };
            events.send(event);
        }
    }
}

type PendingSearch = Option<(
    u64,
    Query,
    Arc<RoaringBitmap>,
    Corpus,
    Sort,
    DateRange,
    GalleryTopology,
    usize,
)>;

fn collect_refresh(
    command: RefreshCommand,
    search: &mut PendingSearch,
    stats: &mut Option<u64>,
    suggest: &mut Option<(u64, String)>,
    patterns: &mut Option<(u64, Vec<TagPattern>)>,
) {
    match command {
        RefreshCommand::Search {
            serial,
            query,
            local_favorites,
            corpus,
            sort,
            dates,
            topology,
            limit,
        } => {
            *search = Some((
                serial,
                query,
                local_favorites,
                corpus,
                sort,
                dates,
                topology,
                limit,
            ));
        }
        RefreshCommand::Stats { serial } => *stats = Some(serial),
        RefreshCommand::Suggest { serial, prefix } => *suggest = Some((serial, prefix)),
        RefreshCommand::ResolvePatterns {
            serial,
            patterns: demand,
        } => *patterns = Some((serial, demand)),
    }
}

fn family_loop(
    booru: Danbooru,
    index: Index,
    gate: RateGate,
    commands: Receiver<FamilyCommand>,
    events: Klaxon,
) {
    while let Ok(mut command) = commands.recv() {
        for newer in commands.try_iter() {
            command = newer;
        }
        match index.family_tree(command.id) {
            Ok(tree) => events.send(Event::Family {
                serial: command.serial,
                tree,
            }),
            Err(err) => {
                events.send(Event::FamilyFault {
                    serial: command.serial,
                    fault: format!("{err:#}"),
                });
                continue;
            }
        }
        match hydrate_family(&booru, &index, &gate, command.id) {
            Ok(true) => match index.family_tree(command.id) {
                Ok(tree) => events.send(Event::Family {
                    serial: command.serial,
                    tree,
                }),
                Err(err) => events.send(Event::FamilyFault {
                    serial: command.serial,
                    fault: format!("{err:#}"),
                }),
            },
            Ok(false) => {}
            Err(err) => events.send(Event::FamilyFault {
                serial: command.serial,
                fault: format!("{err:#}"),
            }),
        }
    }
}

fn definition_loop(
    booru: Danbooru,
    gate: RateGate,
    commands: Receiver<DefinitionCommand>,
    events: Klaxon,
) {
    while let Ok(mut command) = commands.recv() {
        let mut cancelled = Vec::new();
        for newer in commands.try_iter() {
            cancelled.push((command.serial, command.tag));
            command = newer;
        }
        if !cancelled.is_empty() {
            events.send(Event::TagDefinitionsCancelled(cancelled));
        }
        gate.wait();
        let result = booru
            .tag_definitions()
            .context("provider withdrew tag-definition capability")
            .and_then(|source| source.tag_definition(&command.tag))
            .map_err(|err| format!("{err:#}"));
        events.send(Event::TagDefinition {
            serial: command.serial,
            tag: command.tag,
            result,
        });
    }
}

fn hydrate_family(booru: &Danbooru, index: &Index, gate: &RateGate, focus: PostId) -> Result<bool> {
    if index.family_hydrated(focus)? {
        return Ok(false);
    }
    let mut facts = BTreeMap::new();
    let mut cursor = focus;
    let mut root = focus;
    for _ in 0..=4 {
        gate.wait();
        let Some(fact) = booru.kin_single(cursor)? else {
            break;
        };
        root = fact.parent.unwrap_or(fact.id);
        let parent = fact.parent;
        let _old = facts.insert(fact.id, fact);
        let Some(parent) = parent else {
            root = cursor;
            break;
        };
        cursor = parent;
    }

    let mut queue = VecDeque::from([root]);
    let mut visited = HashSet::new();
    let mut fetches = 0;
    while let Some(parent) = queue.pop_front() {
        if !visited.insert(parent) {
            continue;
        }
        let may_have_children = facts.get(&parent).is_none_or(|fact| fact.has_children);
        if !may_have_children {
            continue;
        }
        if fetches == MAX_FAMILY_FETCHES {
            anyhow::bail!("family rooted at #{root} exceeds {MAX_FAMILY_FETCHES} fetches");
        }
        fetches += 1;
        gate.wait();
        for child in booru.kin_children(parent)? {
            if child.has_children {
                queue.push_back(child.id);
            }
            let _old = facts.insert(child.id, child);
        }
    }
    if facts.is_empty() {
        anyhow::bail!("Danbooru returned no kin record for #{focus}");
    }
    let facts = facts.into_values().collect::<Vec<_>>();
    let missing = index.missing_posts(facts.iter().map(|fact| fact.id))?;
    for ids in missing.chunks(FAMILY_POST_BATCH) {
        gate.wait();
        index.absorb_harvest(&booru.posts_by_ids(ids)?)?;
    }
    index.absorb_family(&facts, root)?;
    Ok(true)
}

fn warm_loop(
    booru: Danbooru,
    index: Index,
    gate: RateGate,
    commands: Receiver<WarmCommand>,
    events: Klaxon,
) {
    for command in commands {
        let event = match command {
            WarmCommand::Warm {
                query,
                sort,
                first_page,
                pages,
            } => {
                let query_key = query.to_text();
                match warm(&booru, &index, &gate, query, sort, first_page, pages) {
                    Ok(event) => event,
                    Err(err) => Event::WarmFault {
                        query_key,
                        sort,
                        first_page,
                        fault: format!("{err:#}"),
                    },
                }
            }
            WarmCommand::Refetch(id) => match refetch(&booru, &index, &gate, id) {
                Ok(event) => event,
                Err(err) => Event::Fault(format!("{err:#}")),
            },
        };
        events.send(event);
    }
}

fn media_dispatch(commands: Receiver<MediaCommand>, work: Sender<MediaCommand>) {
    let mut pending = VecDeque::new();
    let mut epoch = BladeEpoch::ROOT;
    while let Some(command) = next_media_command(&commands, &mut pending, &mut epoch) {
        // The offer times out so a command waiting on a saturated pool returns
        // to the queue and stays subject to epoch culling and reprioritization.
        match work.send_timeout(command, MEDIA_OFFER_PATIENCE) {
            Ok(()) => {}
            Err(SendTimeoutError::Timeout(command)) => pending.push_front(command),
            Err(SendTimeoutError::Disconnected(_)) => return,
        }
    }
}

fn media_fetch_loop(media: MediaCache, work: Receiver<MediaCommand>, events: Klaxon) {
    for command in work {
        let event = match command {
            MediaCommand::Blade {
                id, bucket, url, ..
            } => match required_url(url.as_deref()).and_then(|url| media.blade(id, url)) {
                Ok(blade) => Event::Blade { bucket, blade },
                Err(err) => Event::BladeFault {
                    id,
                    bucket,
                    fault: format!("{err:#}"),
                },
            },
            MediaCommand::Cull { .. } => continue,
            MediaCommand::FullBlade {
                id,
                url,
                texture_bound,
            } => match required_url(url.as_deref()).and_then(|url| media.blade(id, url)) {
                Ok(blade) => {
                    let display = blade.downsample(texture_bound);
                    Event::FullBlade { blade, display }
                }
                Err(err) => Event::FullBladeFault {
                    id,
                    fault: format!("{err:#}"),
                },
            },
            MediaCommand::Save { id, url, path } => {
                match required_url(url.as_deref()).and_then(|url| save_media(&media, id, url, path))
                {
                    Ok(path) => Event::MediaSaved { id, path },
                    Err(err) => Event::MediaSaveFault {
                        id,
                        fault: format!("{err:#}"),
                    },
                }
            }
            // Best-effort byte warm into the disk cache: no decode, no event.
            MediaCommand::Prefetch { id, url } => {
                let _warmed = required_url(url.as_deref()).and_then(|url| media.bytes(id, url));
                continue;
            }
        };
        events.send(event);
    }
}

fn save_media(media: &MediaCache, id: PostId, url: &str, path: PathBuf) -> Result<PathBuf> {
    let bytes = media.bytes(id, url)?;
    std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn next_media_command(
    commands: &Receiver<MediaCommand>,
    pending: &mut VecDeque<MediaCommand>,
    epoch: &mut BladeEpoch,
) -> Option<MediaCommand> {
    loop {
        if pending.is_empty() {
            pending.push_back(commands.recv().ok()?);
        }
        pending.extend(
            commands
                .try_iter()
                .take(MEDIA_PENDING_CAPACITY.saturating_sub(pending.len())),
        );
        *epoch = pending
            .iter()
            .filter_map(MediaCommand::blade_epoch)
            .max()
            .unwrap_or(*epoch)
            .max(*epoch);
        pending.retain(|command| command.is_live(*epoch));
        let full = pending
            .iter()
            .position(|command| matches!(command, MediaCommand::FullBlade { .. }));
        let save = pending
            .iter()
            .position(|command| matches!(command, MediaCommand::Save { .. }));
        if let Some(command) = full
            .and_then(|slot| pending.remove(slot))
            .or_else(|| save.and_then(|slot| pending.remove(slot)))
            .or_else(|| pending.pop_front())
        {
            return Some(command);
        }
    }
}

impl MediaCommand {
    fn blade_epoch(&self) -> Option<BladeEpoch> {
        match self {
            Self::Blade { epoch, .. } => Some(*epoch),
            Self::Cull { epoch } => Some(*epoch),
            Self::FullBlade { .. } | Self::Save { .. } | Self::Prefetch { .. } => None,
        }
    }

    fn is_live(&self, epoch: BladeEpoch) -> bool {
        match self {
            Self::Blade {
                epoch: candidate, ..
            } => *candidate >= epoch,
            Self::Cull { .. } => false,
            Self::FullBlade { .. } | Self::Save { .. } | Self::Prefetch { .. } => true,
        }
    }
}

fn crawl_loop(booru: Danbooru, index: Index, gate: RateGate, mirror: MirrorValve, events: Klaxon) {
    loop {
        mirror.await_flow();
        let gap = match crawl_once(&booru, &index, &gate) {
            Ok(event @ Event::Crawled { posts, .. }) => {
                events.murmur(event);
                if posts == 0 {
                    CRAWL_EMPTY_GAP
                } else {
                    CRAWL_GAP
                }
            }
            Ok(event) => {
                events.murmur(event);
                CRAWL_GAP
            }
            Err(err) => {
                events.murmur(Event::Fault(format!("{err:#}")));
                CRAWL_FAULT_GAP
            }
        };
        if !gap.is_zero() {
            thread::sleep(gap);
        }
    }
}

fn merge_loop(index: Index, events: Klaxon) {
    loop {
        let gap = match index.merge_pending_facts(FactMergeBudget::STEADY) {
            Ok(merge) if merge.batches == 0 => MERGE_IDLE_GAP,
            Ok(merge) => {
                events.murmur(Event::FactsMerged {
                    batches: merge.batches,
                    bytes: merge.bytes,
                    groups: merge.groups,
                });
                MERGE_GAP
            }
            Err(err) => {
                events.murmur(Event::Fault(format!("{err:#}")));
                CRAWL_FAULT_GAP
            }
        };
        thread::sleep(gap);
    }
}

fn warm(
    booru: &Danbooru,
    index: &Index,
    gate: &RateGate,
    query: Query,
    sort: Sort,
    first_page: u32,
    pages: u32,
) -> Result<Event> {
    let mut absorbed = 0;
    let pages = pages.max(1);
    let mut fetched = 0;
    let mut exhausted = false;
    for offset in 0..pages {
        let page = first_page + offset;
        gate.wait();
        let posts = booru.posts(&query, sort, page)?;
        fetched += 1;
        if posts.is_empty() {
            exhausted = true;
            break;
        }
        absorbed += posts.len();
        index.absorb_harvest(&posts)?;
    }
    Ok(Event::Warmed {
        query_key: query.to_text(),
        sort,
        first_page,
        pages: fetched,
        posts: absorbed,
        exhausted,
    })
}

fn refetch(booru: &Danbooru, index: &Index, gate: &RateGate, id: PostId) -> Result<Event> {
    gate.wait();
    let posts = booru.single(id)?;
    index.absorb_harvest(&posts)?;
    Ok(Event::Refetched {
        post: posts
            .into_iter()
            .next()
            .map(|harvest| Box::new(harvest.post)),
    })
}

fn crawl_once(booru: &Danbooru, index: &Index, gate: &RateGate) -> Result<Event> {
    let before = index.crawl_before()?;
    gate.wait();
    let posts = booru.crawl_page(before)?;
    let next = posts.iter().map(|harvest| harvest.post.id).min();
    if !posts.is_empty() || next.is_some() {
        index.absorb_crawl_harvest(&posts, next)?;
    }
    Ok(Event::Crawled {
        posts: posts.len(),
        before: next,
    })
}

fn kin_loop(booru: Danbooru, index: Index, gate: RateGate, mirror: MirrorValve, events: Klaxon) {
    loop {
        mirror.await_flow();
        let gap = match kin_once(&booru, &index, &gate) {
            Ok(event @ Event::KinCrawled { complete, .. }) => {
                events.murmur(event);
                if complete {
                    return;
                }
                CRAWL_GAP
            }
            Ok(event) => {
                events.murmur(event);
                CRAWL_GAP
            }
            Err(err) => {
                events.murmur(Event::Fault(format!("{err:#}")));
                CRAWL_FAULT_GAP
            }
        };
        if !gap.is_zero() {
            thread::sleep(gap);
        }
    }
}

fn kin_once(booru: &Danbooru, index: &Index, gate: &RateGate) -> Result<Event> {
    let Backfill::Running(before) = index.kin_backfill()? else {
        return Ok(Event::KinCrawled {
            posts: 0,
            before: None,
            complete: true,
        });
    };
    gate.wait();
    let facts = booru.kin_page(before)?;
    let next = facts.iter().map(|fact| fact.id).min();
    let complete = facts.is_empty();
    index.absorb_kin_crawl(&facts, next, complete)?;
    Ok(Event::KinCrawled {
        posts: facts.len(),
        before: next,
        complete,
    })
}

#[derive(Clone)]
struct RateGate {
    next: Arc<Mutex<Instant>>,
    gap: Duration,
}

impl RateGate {
    fn new(gap: Duration) -> Self {
        Self {
            next: Arc::new(Mutex::new(Instant::now())),
            gap,
        }
    }

    fn wait(&self) {
        let mut next = match self.next.lock() {
            Ok(next) => next,
            Err(poisoned) => poisoned.into_inner(),
        };
        let now = Instant::now();
        if *next > now {
            thread::sleep(*next - now);
        }
        *next = Instant::now() + self.gap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_valve_blocks_and_releases_backfill() -> Result<()> {
        let valve = MirrorValve::new(MirrorPolicy::Paused);
        let witness = valve.clone();
        let (tx, rx) = bounded(1);
        let _waiter = thread::spawn(move || {
            witness.await_flow();
            let _sent = tx.send(());
        });
        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(20)),
            Err(crossbeam_channel::RecvTimeoutError::Timeout)
        ));
        valve.set(MirrorPolicy::Active);
        rx.recv_timeout(Duration::from_secs(1))
            .context("mirror valve did not release")?;
        Ok(())
    }

    #[test]
    fn media_queue_prioritizes_full_blades_and_culls_stale_epochs() -> Result<()> {
        let (tx, rx) = unbounded();
        let stale = BladeEpoch::ROOT.advance();
        let live = stale.advance();
        tx.send(MediaCommand::Blade {
            epoch: stale,
            id: PostId(1),
            bucket: 1,
            url: None,
        })
        .context("send stale blade")?;
        tx.send(MediaCommand::Cull { epoch: live })
            .context("send cull")?;
        tx.send(MediaCommand::Blade {
            epoch: live,
            id: PostId(2),
            bucket: 1,
            url: None,
        })
        .context("send live blade")?;
        tx.send(MediaCommand::FullBlade {
            id: PostId(9),
            url: None,
            texture_bound: [1920, 1080],
        })
        .context("send full blade")?;
        let mut pending = VecDeque::new();
        let mut epoch = BladeEpoch::ROOT;
        let command = next_media_command(&rx, &mut pending, &mut epoch).context("media command")?;
        let MediaCommand::FullBlade { id, .. } = command else {
            anyhow::bail!("expected full blade priority");
        };
        assert_eq!(id, PostId(9));

        let command = next_media_command(&rx, &mut pending, &mut epoch).context("media command")?;
        let MediaCommand::Blade { id, .. } = command else {
            anyhow::bail!("expected live blade after full blade");
        };
        assert_eq!(id, PostId(2));
        assert!(pending.is_empty(), "stale blade survived epoch culling");
        Ok(())
    }
}
