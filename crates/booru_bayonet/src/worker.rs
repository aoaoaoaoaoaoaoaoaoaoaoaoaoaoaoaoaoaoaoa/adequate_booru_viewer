use anyhow::{Context as _, Result};
use crossbeam_channel::{Receiver, Sender, TryIter, unbounded};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use crate::{
    booru::{Booru as _, Danbooru},
    clip::ClipForge,
    index::Index,
    media::{MediaCache, RgbaBlade, required_url},
    model::{Embedding, PostId, PostRecord, Query, Sort},
};

const DANBOORU_READ_GAP: Duration = Duration::from_millis(150);
const CRAWL_GAP: Duration = Duration::ZERO;
const CRAWL_EMPTY_GAP: Duration = Duration::from_mins(1);
const CRAWL_FAULT_GAP: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum Command {
    Warm {
        query: Query,
        sort: Sort,
        pages: u32,
    },
    Blade {
        id: PostId,
        url: Option<String>,
    },
    FullBlade {
        id: PostId,
        url: Option<String>,
    },
    SoftText {
        prompt: String,
    },
    EmbedPosts {
        posts: Vec<PostRecord>,
    },
}

#[derive(Debug)]
pub enum Event {
    Warmed {
        query_key: String,
        posts: usize,
    },
    Crawled {
        posts: usize,
        before: Option<PostId>,
    },
    Blade(RgbaBlade),
    FullBlade(RgbaBlade),
    SoftText {
        prompt: String,
        embedding: Embedding,
    },
    ClipIndexed {
        ids: Vec<PostId>,
        stored: usize,
        faults: usize,
    },
    Fault(String),
}

pub struct Worker {
    io_tx: Sender<Command>,
    clip_tx: Sender<Command>,
    rx: Receiver<Event>,
}

impl Worker {
    pub fn spawn(index: Index, media: MediaCache, model_root: PathBuf) -> Self {
        let (io_tx, io_rx) = unbounded();
        let (clip_tx, clip_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        let read_gate = RateGate::new(DANBOORU_READ_GAP);
        let io_events = event_tx.clone();
        let io_index = index.clone();
        let io_media = media.clone();
        let io_gate = read_gate.clone();
        let _io =
            thread::spawn(move || assault_loop(io_index, io_media, io_gate, io_rx, io_events));
        let crawl_index = index.clone();
        let crawl_events = event_tx.clone();
        let crawl_gate = read_gate.clone();
        let _crawl = thread::spawn(move || crawl_loop(crawl_index, crawl_gate, crawl_events));
        let _clip = thread::spawn(move || clip_loop(index, media, model_root, clip_rx, event_tx));
        Self {
            io_tx,
            clip_tx,
            rx: event_rx,
        }
    }

    pub fn send(&self, command: Command) -> Result<()> {
        match command {
            command
            @ (Command::Warm { .. } | Command::Blade { .. } | Command::FullBlade { .. }) => {
                self.io_tx.send(command).context("send I/O worker command")
            }
            command @ (Command::SoftText { .. } | Command::EmbedPosts { .. }) => self
                .clip_tx
                .send(command)
                .context("send CLIP worker command"),
        }
    }

    pub fn drain(&self) -> TryIter<'_, Event> {
        self.rx.try_iter()
    }
}

fn assault_loop(
    index: Index,
    media: MediaCache,
    gate: RateGate,
    commands: Receiver<Command>,
    events: Sender<Event>,
) {
    let booru = Danbooru::new();
    for command in commands {
        let outcome = match command {
            Command::Warm { query, sort, pages } => warm(&booru, &index, &gate, query, sort, pages),
            Command::Blade { id, url } => required_url(url.as_deref())
                .and_then(|url| media.blade(id, url))
                .map(Event::Blade),
            Command::FullBlade { id, url } => required_url(url.as_deref())
                .and_then(|url| media.blade(id, url))
                .map(Event::FullBlade),
            Command::SoftText { .. } | Command::EmbedPosts { .. } => {
                Ok(Event::Fault("CLIP command reached I/O worker".to_owned()))
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
            Command::SoftText { prompt } => forge(&mut clip, &model_root).and_then(|clip| {
                clip.text(&prompt)
                    .map(|embedding| Event::SoftText { prompt, embedding })
            }),
            Command::EmbedPosts { posts } => {
                forge(&mut clip, &model_root).map(|clip| embed_posts(&index, &media, clip, posts))
            }
            Command::Warm { .. } | Command::Blade { .. } | Command::FullBlade { .. } => {
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

fn warm(
    booru: &Danbooru,
    index: &Index,
    gate: &RateGate,
    query: Query,
    sort: Sort,
    pages: u32,
) -> Result<Event> {
    let mut absorbed = 0;
    for page in 1..=pages.max(1) {
        gate.wait();
        let posts = booru.posts(&query, sort, page)?;
        absorbed += posts.len();
        index.absorb(&posts)?;
    }
    Ok(Event::Warmed {
        query_key: query.key(),
        posts: absorbed,
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
        *clip = Some(ClipForge::new(model_root.to_path_buf())?);
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
    let url = required_url(post.blade_url())?;
    let bytes = media.bytes(post.id, url)?;
    let embedding = clip.image(&bytes)?;
    index.put_embedding(post.id, &embedding)?;
    Ok(true)
}
