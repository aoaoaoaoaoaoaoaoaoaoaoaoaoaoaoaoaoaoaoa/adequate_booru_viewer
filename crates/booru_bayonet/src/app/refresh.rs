use super::*;

impl Bayonet {
    pub(super) fn reap(&mut self, warm: bool, pages: u32) {
        startup("app.reap.enter");
        let query = self.query();
        self.request_refresh();
        if warm {
            startup("app.reap.warm.enter");
            if let Err(err) = self.dispatch_warm(query, pages) {
                self.status = format!("{err:#}");
            }
            startup("app.reap.warm.done");
        }
        startup("app.reap.stats.enter");
        self.request_stats();
        startup("app.reap.stats.done");
    }

    pub(super) fn strike(&mut self, warm: bool, pages: u32) {
        self.reap(warm, pages);
    }

    pub(super) fn request_refresh(&mut self) {
        let serial = next_serial(&mut self.refresh_serial);
        match self.refresh_pulse {
            AsyncPulse::Idle => self.dispatch_refresh(serial),
            AsyncPulse::InFlight { serial: inflight } | AsyncPulse::Dirty { serial: inflight } => {
                self.refresh_pulse = AsyncPulse::Dirty { serial: inflight };
            }
        }
    }

    fn dispatch_refresh(&mut self, serial: u64) {
        let magnet = self.rank_needle().map(|needle| MagnetRefresh {
            needle,
            alpha: self.rank_alpha,
            limit: RESULT_LIMIT,
            pool: MAGNET_POOL,
            backlog: MAGNET_BACKLOG,
        });
        let send = self.worker.send(Command::Refresh {
            serial,
            query: self.query(),
            sort: self.sort,
            limit: RESULT_LIMIT,
            magnet,
        });
        match send {
            Ok(()) => {
                self.refresh_pulse = AsyncPulse::InFlight { serial };
                "search refreshing".clone_into(&mut self.status);
            }
            Err(err) => {
                self.refresh_pulse = AsyncPulse::Idle;
                self.status = format!("{err:#}");
            }
        }
    }

    pub(super) fn finish_refresh(
        &mut self,
        serial: u64,
        hit: Option<RefreshHit>,
        ctx: &egui::Context,
    ) {
        let Some(inflight) = self.refresh_pulse.inflight_serial() else {
            return;
        };
        if inflight != serial {
            return;
        }
        let dirty = self.refresh_pulse.is_dirty();
        self.refresh_pulse = AsyncPulse::Idle;
        if !dirty
            && serial == self.refresh_serial
            && let Some(hit) = hit
        {
            self.install_refresh(hit);
            ctx.request_repaint();
        }
        if dirty {
            self.dispatch_refresh(self.refresh_serial);
        }
    }

    fn install_refresh(&mut self, hit: RefreshHit) {
        match hit {
            RefreshHit::Hard(hit) => self.install_hard_refresh(hit),
            RefreshHit::Magnetic(hit) => {
                let queued = self.queue_embeddings(hit.missing);
                let posts = hit.hit.posts.len();
                let candidates = hit.hit.candidates;
                let embedded = hit.embedded;
                let pool = hit.pool;
                self.install_hit(hit.hit);
                self.status = format!(
                    "{} hits from {} candidates; magnet rank {}/{} embedded, queued {}; α {:.2}",
                    posts, candidates, embedded, pool, queued, self.rank_alpha
                );
            }
        }
    }

    fn install_hard_refresh(&mut self, hit: SearchHit) {
        let rank_armed = self.rank_alpha > 0.0 && !self.rank_magnets.is_empty();
        let posts = hit.posts.len();
        let candidates = hit.candidates;
        self.install_hit(hit);
        self.status = if rank_armed {
            format!(
                "{} hits from {} candidates; waiting for magnet image embeddings",
                posts, candidates
            )
        } else {
            format!(
                "{} hits from {} candidates; {}",
                posts,
                candidates,
                compact_path(&self.lair.data)
            )
        };
    }

    pub(super) fn request_stats(&mut self) {
        let serial = next_serial(&mut self.stats_serial);
        match self.stats_pulse {
            AsyncPulse::Idle => self.dispatch_stats(serial),
            AsyncPulse::InFlight { serial: inflight } | AsyncPulse::Dirty { serial: inflight } => {
                self.stats_pulse = AsyncPulse::Dirty { serial: inflight };
            }
        }
    }

    fn dispatch_stats(&mut self, serial: u64) {
        match self.worker.send(Command::Stats { serial }) {
            Ok(()) => self.stats_pulse = AsyncPulse::InFlight { serial },
            Err(err) => {
                self.stats_pulse = AsyncPulse::Idle;
                self.cache_status = format!("cache stats fault: {err:#}");
            }
        }
    }

    pub(super) fn finish_stats(
        &mut self,
        serial: u64,
        stats: Option<CacheStats>,
        ctx: &egui::Context,
    ) {
        let Some(inflight) = self.stats_pulse.inflight_serial() else {
            return;
        };
        if inflight != serial {
            return;
        }
        let dirty = self.stats_pulse.is_dirty();
        self.stats_pulse = AsyncPulse::Idle;
        if !dirty
            && serial == self.stats_serial
            && let Some(stats) = stats
        {
            self.cache_status = cache_status(&stats);
            ctx.request_repaint();
        }
        if dirty {
            self.dispatch_stats(self.stats_serial);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AsyncPulse {
    Idle,
    InFlight { serial: u64 },
    Dirty { serial: u64 },
}

impl AsyncPulse {
    pub(super) fn inflight_serial(self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::InFlight { serial } | Self::Dirty { serial } => Some(serial),
        }
    }

    fn is_dirty(self) -> bool {
        matches!(self, Self::Dirty { .. })
    }
}

fn next_serial(serial: &mut u64) -> u64 {
    *serial = serial.saturating_add(1);
    *serial
}

fn cache_status(stats: &CacheStats) -> String {
    let ratings = stats
        .ratings
        .iter()
        .map(|(rating, posts)| format!("{}:{posts}", rating.key()))
        .collect::<Vec<_>>()
        .join("/");
    let frontier = match (stats.crawl_before, stats.rough_crawl_percent()) {
        (Some(before), Some(percent)) => format!("crawl≤#{before} ≈{percent:.1}% ID"),
        (Some(before), None) => format!("crawl≤#{before}"),
        (None, _) => "crawl unstarted".to_owned(),
    };
    let newest = stats
        .newest
        .map_or_else(|| "newest unknown".to_owned(), |id| format!("newest #{id}"));
    format!(
        "cache {} posts, {} tag chunks, {} dino, {} pending fact batches, ratings {ratings}, {newest}, {frontier}",
        stats.posts, stats.tag_chunks, stats.embeddings, stats.pending_fact_batches
    )
}
