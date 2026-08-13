use super::*;

/// Crawl pages land every 150 ms; re-running the full search and decoding four
/// rating lanes at that cadence is pure background burn. Gate both to a human
/// cadence and let `flush_pulse_gates` fire the trailing edge.
const REFRESH_GAP: Duration = Duration::from_millis(500);
const STATS_GAP: Duration = Duration::from_millis(1_500);

impl Bayonet {
    pub(super) fn strike(&mut self, warm: bool, pages: u32) {
        startup("app.strike.enter");
        let query = self.query.clone();
        self.request_refresh();
        if warm {
            startup("app.strike.warm.enter");
            if let Err(err) = self.dispatch_warm(query, pages) {
                self.status = format!("{err:#}");
            }
            startup("app.strike.warm.done");
        }
        startup("app.strike.stats.enter");
        self.request_stats();
        startup("app.strike.stats.done");
    }

    /// Throttled refresh for background events (crawl, warm, embeddings).
    /// User actions call `request_refresh` directly and are never gated.
    pub(super) fn nudge_refresh(&mut self) {
        if self.refresh_gate.nudge() {
            self.request_refresh();
        }
    }

    pub(super) fn nudge_stats(&mut self) {
        if self.stats_gate.nudge() {
            self.request_stats();
        }
    }

    pub(super) fn flush_pulse_gates(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.refresh_gate.flush(now) {
            self.request_refresh();
            changed = true;
        }
        if self.stats_gate.flush(now) {
            self.request_stats();
            changed = true;
        }
        changed
    }

    pub(super) fn pulse_gate_deadline(&self) -> Option<Instant> {
        [self.refresh_gate.deadline(), self.stats_gate.deadline()]
            .into_iter()
            .flatten()
            .min()
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
        let send = self.worker.send(Command::Refresh {
            serial,
            query: self.query.clone(),
            local_favorites: self.local_favorites.snapshot(),
            corpus: self.filter_selection.corpus(),
            sort: self.sort,
            dates: self.date_range.normalized(),
            topology: self.gallery,
            limit: self.retrieval_horizon,
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
        hit: Option<SearchHit>,
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

    fn install_refresh(&mut self, hit: SearchHit) {
        let posts = hit.posts.len();
        let candidates = hit.candidates;
        let tail = hit.tail;
        self.horizon_pending = hit.horizon < self.retrieval_horizon;
        self.hit_cache.put(
            HitKey::new(
                &self.query,
                self.filter_selection.corpus(),
                self.sort,
                self.date_range,
                self.gallery,
            ),
            hit.clone(),
        );
        self.install_hit(hit);
        self.status = match tail {
            SearchTail::Open => format!(
                "{posts} loaded from {candidates} matching posts; scroll for more; {}",
                self.lair.data.display()
            ),
            SearchTail::Exhausted => format!(
                "{posts} hits from {candidates} matching posts; {}",
                self.lair.data.display()
            ),
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
        if let Some(stats) = stats {
            self.cache_status = cache_status(&stats);
            ctx.request_repaint();
        }
        if dirty {
            self.dispatch_stats(self.stats_serial);
        }
    }
}

/// Rate gate with a trailing edge: suppressed nudges fire once the gap elapses.
#[derive(Clone, Copy, Debug)]
pub(super) struct PulseGate {
    gap: Duration,
    last: Option<Instant>,
    stale: bool,
}

impl PulseGate {
    pub(super) fn refresh() -> Self {
        Self::new(REFRESH_GAP)
    }

    pub(super) fn stats() -> Self {
        Self::new(STATS_GAP)
    }

    fn new(gap: Duration) -> Self {
        Self {
            gap,
            last: None,
            stale: false,
        }
    }

    fn ready(&self, now: Instant) -> bool {
        self.deadline().is_none_or(|deadline| deadline <= now)
    }

    fn fire(&mut self) {
        self.last = Some(Instant::now());
        self.stale = false;
    }

    fn nudge(&mut self) -> bool {
        if self.ready(Instant::now()) {
            self.fire();
            true
        } else {
            self.stale = true;
            false
        }
    }

    fn flush(&mut self, now: Instant) -> bool {
        if self.stale && self.ready(now) {
            self.fire();
            true
        } else {
            false
        }
    }

    fn deadline(&self) -> Option<Instant> {
        let last = self.last.filter(|_| self.stale)?;
        last.checked_add(self.gap)
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
        "cache {} posts, {} tag chunks, {} pending fact batches, ratings {ratings}, {newest}, {frontier}",
        stats.posts, stats.tag_chunks, stats.pending_fact_batches
    )
}
