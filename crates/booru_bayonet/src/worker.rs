use anyhow::{Context as _, Result};
use crossbeam_channel::{Receiver, Sender, TryIter, unbounded};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use crate::{
    booru::{Booru as _, Danbooru},
    clip::ClipForge,
    index::{CacheStats, FactMergeBudget, Index, SoftHit},
    media::{MediaCache, RgbaBlade, required_url},
    model::{Embedding, PostId, PostRecord, Query, SearchHit, Sort},
};

const DANBOORU_READ_GAP: Duration = Duration::from_millis(150);
const CRAWL_GAP: Duration = Duration::ZERO;
const CRAWL_EMPTY_GAP: Duration = Duration::from_mins(1);
const CRAWL_FAULT_GAP: Duration = Duration::from_secs(5);
const MERGE_GAP: Duration = Duration::from_millis(250);
const MERGE_IDLE_GAP: Duration = Duration::from_secs(2);

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
        sort: Sort,
        limit: usize,
        soft: Option<SoftRefresh>,
    },
    Stats {
        serial: u64,
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
    },
    SaveMedia {
        id: PostId,
        url: Option<String>,
        path: PathBuf,
    },
    EmbedPosts {
        posts: Vec<PostRecord>,
    },
}

#[derive(Debug)]
pub enum Event {
    Refreshed {
        serial: u64,
        hit: RefreshHit,
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
    Warmed {
        query_key: String,
        sort: Sort,
        first_page: u32,
        pages: u32,
        posts: usize,
        exhausted: bool,
    },
    Crawled {
        posts: usize,
        before: Option<PostId>,
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
    FullBlade(RgbaBlade),
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
    ClipIndexed {
        ids: Vec<PostId>,
        stored: usize,
        faults: usize,
    },
    FactsMerged {
        batches: usize,
        bytes: usize,
        groups: usize,
    },
    Fault(String),
}

#[derive(Clone, Debug)]
pub struct SoftRefresh {
    pub needle: Embedding,
    pub alpha: f32,
    pub limit: usize,
    pub pool: usize,
    pub backlog: usize,
}

#[derive(Clone, Debug)]
pub enum RefreshHit {
    Hard(SearchHit),
    Soft(SoftHit),
}

pub struct Worker {
    refresh_tx: Sender<RefreshCommand>,
    warm_tx: Sender<WarmCommand>,
    media_tx: Sender<MediaCommand>,
    clip_tx: Sender<Command>,
    rx: Receiver<Event>,
}

impl Worker {
    pub fn spawn(index: Index, media: MediaCache, model_root: PathBuf) -> Self {
        let (refresh_tx, refresh_rx) = unbounded();
        let (warm_tx, warm_rx) = unbounded();
        let (media_tx, media_rx) = unbounded();
        let (clip_tx, clip_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        let refresh_events = event_tx.clone();
        let refresh_index = index.clone();
        let _refresh =
            thread::spawn(move || refresh_loop(refresh_index, refresh_rx, refresh_events));
        let read_gate = RateGate::new(DANBOORU_READ_GAP);
        let warm_events = event_tx.clone();
        let warm_index = index.clone();
        let warm_gate = read_gate.clone();
        let _warm = thread::spawn(move || warm_loop(warm_index, warm_gate, warm_rx, warm_events));
        let media_events = event_tx.clone();
        let media_cache = media.clone();
        let _media = thread::spawn(move || media_loop(media_cache, media_rx, media_events));
        let crawl_index = index.clone();
        let crawl_events = event_tx.clone();
        let crawl_gate = read_gate.clone();
        let _crawl = thread::spawn(move || crawl_loop(crawl_index, crawl_gate, crawl_events));
        let merge_index = index.clone();
        let merge_events = event_tx.clone();
        let _merge = thread::spawn(move || merge_loop(merge_index, merge_events));
        let _clip = thread::spawn(move || clip_loop(index, media, model_root, clip_rx, event_tx));
        Self {
            refresh_tx,
            warm_tx,
            media_tx,
            clip_tx,
            rx: event_rx,
        }
    }

    pub fn send(&self, command: Command) -> Result<()> {
        match command {
            Command::Refresh {
                serial,
                query,
                sort,
                limit,
                soft,
            } => self
                .refresh_tx
                .send(RefreshCommand::Search {
                    serial,
                    query,
                    sort,
                    limit,
                    soft,
                })
                .context("send refresh worker command"),
            Command::Stats { serial } => self
                .refresh_tx
                .send(RefreshCommand::Stats { serial })
                .context("send stats worker command"),
            Command::Warm {
                query,
                sort,
                first_page,
                pages,
            } => self
                .warm_tx
                .send(WarmCommand::Warm {
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
                .send(MediaCommand::Blade {
                    epoch,
                    id,
                    bucket,
                    url,
                })
                .context("send media worker command"),
            Command::CullBlades { epoch } => self
                .media_tx
                .send(MediaCommand::Cull { epoch })
                .context("send media worker command"),
            Command::FullBlade { id, url } => self
                .media_tx
                .send(MediaCommand::FullBlade { id, url })
                .context("send media worker command"),
            Command::SaveMedia { id, url, path } => self
                .media_tx
                .send(MediaCommand::Save { id, url, path })
                .context("send media worker command"),
            command @ Command::EmbedPosts { .. } => self
                .clip_tx
                .send(command)
                .context("send CLIP worker command"),
        }
    }

    pub fn drain(&self) -> TryIter<'_, Event> {
        self.rx.try_iter()
    }
}

#[derive(Debug)]
enum RefreshCommand {
    Search {
        serial: u64,
        query: Query,
        sort: Sort,
        limit: usize,
        soft: Option<SoftRefresh>,
    },
    Stats {
        serial: u64,
    },
}

#[derive(Debug)]
enum WarmCommand {
    Warm {
        query: Query,
        sort: Sort,
        first_page: u32,
        pages: u32,
    },
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
    },
    Save {
        id: PostId,
        url: Option<String>,
        path: PathBuf,
    },
}

fn refresh_loop(index: Index, commands: Receiver<RefreshCommand>, events: Sender<Event>) {
    while let Ok(first) = commands.recv() {
        let mut search = None;
        let mut stats = None;
        collect_refresh(first, &mut search, &mut stats);
        for command in commands.try_iter() {
            collect_refresh(command, &mut search, &mut stats);
        }
        if let Some((serial, query, sort, limit, soft)) = search {
            let event = match refresh(&index, &query, sort, limit, soft) {
                Ok(hit) => Event::Refreshed { serial, hit },
                Err(err) => Event::RefreshFault {
                    serial,
                    fault: format!("{err:#}"),
                },
            };
            let _sent = events.send(event);
        }
        if let Some(serial) = stats {
            let event = match index.stats() {
                Ok(stats) => Event::Stats { serial, stats },
                Err(err) => Event::StatsFault {
                    serial,
                    fault: format!("{err:#}"),
                },
            };
            let _sent = events.send(event);
        }
    }
}

type PendingSearch = Option<(u64, Query, Sort, usize, Option<SoftRefresh>)>;

fn collect_refresh(command: RefreshCommand, search: &mut PendingSearch, stats: &mut Option<u64>) {
    match command {
        RefreshCommand::Search {
            serial,
            query,
            sort,
            limit,
            soft,
        } => *search = Some((serial, query, sort, limit, soft)),
        RefreshCommand::Stats { serial } => *stats = Some(serial),
    }
}

fn refresh(
    index: &Index,
    query: &Query,
    sort: Sort,
    limit: usize,
    soft: Option<SoftRefresh>,
) -> Result<RefreshHit> {
    match soft {
        Some(soft) => index
            .search_soft(
                query,
                sort,
                &soft.needle,
                soft.alpha,
                soft.limit,
                soft.pool,
                soft.backlog,
            )
            .map(RefreshHit::Soft),
        None => index.search(query, sort, limit).map(RefreshHit::Hard),
    }
}

fn warm_loop(index: Index, gate: RateGate, commands: Receiver<WarmCommand>, events: Sender<Event>) {
    let booru = Danbooru::new();
    for command in commands {
        let outcome = match command {
            WarmCommand::Warm {
                query,
                sort,
                first_page,
                pages,
            } => warm(&booru, &index, &gate, query, sort, first_page, pages),
        };
        match outcome {
            Ok(event) => {
                let _sent = events.send(event);
            }
            Err(err) => {
                let _sent = events.send(Event::Fault(format!("{err:#}")));
            }
        }
    }
}

fn media_loop(media: MediaCache, commands: Receiver<MediaCommand>, events: Sender<Event>) {
    let mut pending = VecDeque::new();
    let mut epoch = BladeEpoch::ROOT;
    while let Some(command) = next_media_command(&commands, &mut pending, &mut epoch) {
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
            MediaCommand::FullBlade { id, url } => {
                match required_url(url.as_deref()).and_then(|url| media.blade(id, url)) {
                    Ok(blade) => Event::FullBlade(blade),
                    Err(err) => Event::FullBladeFault {
                        id,
                        fault: format!("{err:#}"),
                    },
                }
            }
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
        };
        let _sent = events.send(event);
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
        pending.extend(commands.try_iter());
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
            Self::FullBlade { .. } | Self::Save { .. } => None,
        }
    }

    fn is_live(&self, epoch: BladeEpoch) -> bool {
        match self {
            Self::Blade {
                epoch: candidate, ..
            } => *candidate >= epoch,
            Self::Cull { .. } => false,
            Self::FullBlade { .. } | Self::Save { .. } => true,
        }
    }
}

fn clip_loop(
    index: Index,
    media: MediaCache,
    model_root: PathBuf,
    commands: Receiver<Command>,
    events: Sender<Event>,
) {
    let mut clip = None;
    for command in commands {
        let outcome = match command {
            Command::EmbedPosts { posts } => {
                forge(&mut clip, &model_root).map(|clip| embed_posts(&index, &media, clip, posts))
            }
            Command::Warm { .. }
            | Command::Refresh { .. }
            | Command::Stats { .. }
            | Command::Blade { .. }
            | Command::CullBlades { .. }
            | Command::FullBlade { .. }
            | Command::SaveMedia { .. } => {
                Ok(Event::Fault("I/O command reached CLIP worker".to_owned()))
            }
        };
        match outcome {
            Ok(event) => {
                let _sent = events.send(event);
            }
            Err(err) => {
                let _sent = events.send(Event::Fault(format!("{err:#}")));
            }
        }
    }
}

fn crawl_loop(index: Index, gate: RateGate, events: Sender<Event>) {
    let booru = Danbooru::new();
    loop {
        let gap = match crawl_once(&booru, &index, &gate) {
            Ok(event @ Event::Crawled { posts, .. }) => {
                let _sent = events.send(event);
                if posts == 0 {
                    CRAWL_EMPTY_GAP
                } else {
                    CRAWL_GAP
                }
            }
            Ok(event) => {
                let _sent = events.send(event);
                CRAWL_GAP
            }
            Err(err) => {
                let _sent = events.send(Event::Fault(format!("{err:#}")));
                CRAWL_FAULT_GAP
            }
        };
        if !gap.is_zero() {
            thread::sleep(gap);
        }
    }
}

fn merge_loop(index: Index, events: Sender<Event>) {
    loop {
        let gap = match index.merge_pending_facts(FactMergeBudget::STEADY) {
            Ok(merge) if merge.batches == 0 => MERGE_IDLE_GAP,
            Ok(merge) => {
                let _sent = events.send(Event::FactsMerged {
                    batches: merge.batches,
                    bytes: merge.bytes,
                    groups: merge.groups,
                });
                MERGE_GAP
            }
            Err(err) => {
                let _sent = events.send(Event::Fault(format!("{err:#}")));
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
        index.absorb(&posts)?;
    }
    Ok(Event::Warmed {
        query_key: query.key(),
        sort,
        first_page,
        pages: fetched,
        posts: absorbed,
        exhausted,
    })
}

fn crawl_once(booru: &Danbooru, index: &Index, gate: &RateGate) -> Result<Event> {
    let before = index.crawl_before()?;
    gate.wait();
    let posts = booru.crawl_page(before)?;
    let next = posts.iter().map(|post| post.id).min();
    if !posts.is_empty() {
        index.absorb(&posts)?;
    }
    if let Some(next) = next {
        index.set_crawl_before(next)?;
    }
    Ok(Event::Crawled {
        posts: posts.len(),
        before: next,
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

fn forge<'a>(clip: &'a mut Option<ClipForge>, model_root: &Path) -> Result<&'a mut ClipForge> {
    if clip.is_none() {
        *clip = Some(ClipForge::new(model_root.to_path_buf()));
    }
    clip.as_mut().context("CLIP forge missing")
}

fn embed_posts(
    index: &Index,
    media: &MediaCache,
    clip: &mut ClipForge,
    posts: Vec<PostRecord>,
) -> Event {
    let mut ids = Vec::with_capacity(posts.len());
    let mut stored = 0_usize;
    let mut faults = 0_usize;
    for post in posts {
        ids.push(post.id);
        match embed_post(index, media, clip, &post) {
            Ok(true) => stored += 1,
            Ok(false) => {}
            Err(_err) => faults += 1,
        }
    }
    Event::ClipIndexed {
        ids,
        stored,
        faults,
    }
}

fn embed_post(
    index: &Index,
    media: &MediaCache,
    clip: &mut ClipForge,
    post: &PostRecord,
) -> Result<bool> {
    if index.has_embedding(post.id)? {
        return Ok(false);
    }
    let url = required_url(post.clip_url())?;
    let bytes = media.bytes(post.id, url)?;
    let embedding = clip.image(&bytes)?;
    index.put_embedding(post.id, &embedding)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_queue_culls_stale_thumbnail_epochs() -> Result<()> {
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
        let mut pending = VecDeque::new();
        let mut epoch = BladeEpoch::ROOT;
        let command = next_media_command(&rx, &mut pending, &mut epoch).context("media command")?;
        let MediaCommand::Blade { id, .. } = command else {
            anyhow::bail!("expected live blade after cull");
        };
        assert_eq!(id, PostId(2));
        Ok(())
    }

    #[test]
    fn media_queue_prioritizes_full_blades() -> Result<()> {
        let (tx, rx) = unbounded();
        let epoch = BladeEpoch::ROOT.advance();
        tx.send(MediaCommand::Blade {
            epoch,
            id: PostId(1),
            bucket: 1,
            url: None,
        })
        .context("send blade")?;
        tx.send(MediaCommand::FullBlade {
            id: PostId(9),
            url: None,
        })
        .context("send full blade")?;
        let mut pending = VecDeque::new();
        let mut epoch = BladeEpoch::ROOT;
        let command = next_media_command(&rx, &mut pending, &mut epoch).context("media command")?;
        let MediaCommand::FullBlade { id, .. } = command else {
            anyhow::bail!("expected full blade priority");
        };
        assert_eq!(id, PostId(9));
        Ok(())
    }
}
