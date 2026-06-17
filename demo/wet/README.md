# wet demo

`cargo xtask wet-demo` — the canonical "exercise the features, show off the
liquid" take. The demo stages only `config.toml` and `slate.toml` (the pinned
fixture: a `work`/`play` library and the pre-record slate); the durable
Danbooru index and media cache come from the operator's normal XDG data/cache
roots. That is the intended covenant: reproducible choreography, not a fake
aquarium.

## Segments

The take is cut into independently-reproducible segments, listed in order by
`segments.toml` and stored under `segments/<name>.toml`. Each seam is a rest
point (nothing zoomed, entry field empty, scroll at the top), so a segment's
entry state is an exact relaunch — that is what `shutters` (persisted recess
folds) buys us.

- `--scaffold` — replay every segment in order and, at each seam, snapshot the
  app's live `slate`+`config` into the next segment's entry state
  (`segments/<name>.{slate,config}.toml`). Run this first; it regenerates all
  downstream entry states from the app's own writer. A divergence between a
  `--segment` replay and the continuous run is a hole in our serialization.
- `--segment <name>` — record one segment in isolation from its (scaffolded)
  entry state, into `abv-wet-segment-<name>.mp4`. The fast loop for tuning one
  beat's mouse work.
- (no flag) — the continuous final take: every segment back-to-back from
  segment 01's entry, one recording, so the waves never reset.

`--dry-run` prints the plan (per-segment step counts and durations) without
launching anything.

Segment 01's entry is the hand-authored base (`config.toml` + `slate.toml`);
segments 02+ entries are scaffold-generated and not hand-edited.
