# adequate booru viewer — design

Goal: a native, anonymous, read-only Danbooru reference workbench whose warm-cache interaction path is local index math, not live HTTP search.

## Storage Contract

The objection that long scraping work must survive restart is correct. The app treats index and media differently:

- durable index: `ProjectDirs::data_local_dir()/index.redb`
- disposable media cache: `ProjectDirs::cache_dir()/media`
- user intent: `ProjectDirs::config_dir()/config.toml` — saved filters,
  folders, preferences; only things a person would write by hand
- workbench state: `ProjectDirs::state_dir()/slate.toml` (data dir fallback
  off Linux) — scratch query, active filter, sort, density, folder
  collapse; the app's snapshot of itself, free to decay to defaults

The durable database persists both directions:

- forward: `post_id → PostRecord`
- reverse: `tag → roaring(post_id)`
- rating lane: `rating → roaring(post_id)`
- sort lanes: score and favorite indexes, with post-id ordering as the newest lane
- crawl cursor: latest Danbooru passive-crawl `page=b<post_id>` frontier

Posts tagged `animated` are outside the reference-workbench contract, as are posts the API serves with every media URL stripped (gold-walled or banned). The ingestion path refuses to insert either, re-absorption purges already-cached offenders from the forward table and tag/rating/sort lanes, and search hydration skips any media-less stragglers in the meantime.

Startup restores both files before the first local search. The split is a contract: losing the slate must never lose user intent, so the slate loader decays silently to defaults while a corrupt config fails loudly.

## Query Path

Warm-cache filtering is recursive bitmap algebra over persisted `roaring` sets. The query is a tree of atom leaves (`tag` and `rating:*`) plus `AND`, `OR`, `XOR`/select, and unary `NOT`; textual `-tag` is only input sugar for `NOT tag`. `AND` intersects, `OR` unions, `XOR` keeps posts present in exactly one child, and `NOT` subtracts from the cached post universe. Sorting is either an ordered lane walk for broad sets or a bounded local candidate sort for smaller intersections. UI query changes do not hit the network.

Danbooru HTTP is only an anonymous read-only ingress. The worker calls `GET /posts.json`; no login, API key, write endpoint, vote endpoint, or mutation primitive exists in the code.

A passive crawler walks Danbooru newest-to-oldest with `page=b<id>` and a durable cursor. The active query warmer separately walks page 1, 2, 3, ... for the current query/sort until exhaustion or Danbooru's anonymous 1000-page search cap, then re-runs local search as pages are absorbed, so score/favorite sorts keep widening while the cache warms. Both paths share one 150 ms read gate, about 6.7 requests/sec against Danbooru's documented 10 requests/sec read ceiling.

The UI reports cache status directly: indexed posts, tag keys, per-rating counts, newest known post, and the passive crawl frontier. The displayed crawl percentage is an ID-space estimate, not a claim about undeleted corpus cardinality.

## Multi-Tag Reality

Danbooru is the ingestion oracle, but not the interaction engine. Live anonymous search can be tag-count constrained; local search is not. The current warmer extracts anonymous-safe positive atoms from the boolean tree, sends at most one rating atom and enough positive tag atoms to stay under Danbooru's practical tag budget plus an order metatag, absorbs the resulting posts, and then the local index enforces the full tree. Later crawlers should choose the rarest locally-known positive atoms as the remote seed.

## Pure Rust UI

The UI is `egui` on a bespoke `winit`/`wgpu` integration (`boiler.rs`) — no eframe. Owning the event loop means worker events wake the UI directly, and owning the render graph means arbitrary GPU passes: when the full-image viewer opens, the UI renders to an offscreen texture, the `dwemer_poolrooms` compositor runs its dual-Kawase veil and persistent water field, and an SDF-masked composite keeps the viewer window sharp. With water disabled, egui rasterizes straight into the swapchain at zero added cost. There is no JavaScript surface. Background threads perform network and decode work, then send decoded RGBA blades to the UI thread for texture upload.

The main grid is image-only for scan speed. Filter state lives in a left boolean-tree panel. One group is active; tag entry, autocomplete, and hover-palette mutations target that group. Group frames are color coded, selectable, nest arbitrarily, and expose `AND`/`OR`/`XOR`/`NOT` controls. Thumbnails expose tag mutation on hover (`-` inserts `NOT tag`, `+` inserts `tag`, `×` removes existing occurrences). Clicking a thumbnail opens a scaled full-image frame with copy and right-click-close.

`Ctrl` + mouse-wheel scales the grid from half-size to triple-size. Danbooru currently exposes media variants named `180x180`, `360x360`, `720x720`, `sample`, and `original`; the viewer stores the 180/360/720 URLs when present and chooses the thumbnail bucket from the current tile edge. The full-image frame uses sample/original fallbacks.

## Future Boorus

The seam is `Booru::posts(Query, Sort, page) -> Vec<PostRecord>`. Other boorus should map their wire format into the canonical `PostRecord`, then reuse the same index, sort lanes, media cache, and UI.
