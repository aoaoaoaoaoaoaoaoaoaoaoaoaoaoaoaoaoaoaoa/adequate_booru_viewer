use anyhow::{Context as _, Result};
use crossbeam_channel::{Receiver, Sender, TryIter, unbounded};
use std::{thread, time::Duration};

use crate::{
    booru::{Booru as _, Danbooru},
    index::Index,
    media::{MediaCache, RgbaBlade, required_url},
    model::{Query, Sort},
};

#[derive(Debug)]
pub enum Command {
    Warm {
        query: Query,
        sort: Sort,
        pages: u32,
    },
    Blade {
        id: crate::model::PostId,
        url: Option<String>,
    },
}

#[derive(Debug)]
pub enum Event {
    Warmed { query_key: String, posts: usize },
    Blade(RgbaBlade),
    Fault(String),
}

pub struct Worker {
    tx: Sender<Command>,
    rx: Receiver<Event>,
}

impl Worker {
    pub fn spawn(index: Index, media: MediaCache) -> Self {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        let _handle = thread::spawn(move || assault_loop(index, media, command_rx, event_tx));
        Self {
            tx: command_tx,
            rx: event_rx,
        }
    }

    pub fn send(&self, command: Command) -> Result<()> {
        self.tx.send(command).context("send worker command")
    }

    pub fn drain(&self) -> TryIter<'_, Event> {
        self.rx.try_iter()
    }
}

fn assault_loop(
    index: Index,
    media: MediaCache,
    commands: Receiver<Command>,
    events: Sender<Event>,
) {
    let booru = Danbooru::new();
    for command in commands {
        let outcome = match command {
            Command::Warm { query, sort, pages } => warm(&booru, &index, query, sort, pages),
            Command::Blade { id, url } => required_url(url.as_deref())
                .and_then(|url| media.blade(id, url))
                .map(Event::Blade),
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

fn warm(booru: &Danbooru, index: &Index, query: Query, sort: Sort, pages: u32) -> Result<Event> {
    let mut absorbed = 0;
    for page in 1..=pages.max(1) {
        let posts = booru.posts(&query, sort, page)?;
        absorbed += posts.len();
        index.absorb(&posts)?;
        thread::sleep(Duration::from_millis(150));
    }
    Ok(Event::Warmed {
        query_key: query.key(),
        posts: absorbed,
    })
}
