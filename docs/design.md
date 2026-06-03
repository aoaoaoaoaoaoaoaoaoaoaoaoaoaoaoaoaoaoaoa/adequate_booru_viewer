# Booru Bayonet Design

Goal: a native, anonymous, read-only Danbooru reference workbench whose warm-cache interaction path is local index math, not live HTTP search.

## Storage Contract

The objection that long scraping work must survive restart is correct. The app treats index and media differently:

- durable index: `ProjectDirs::data_local_dir()/index.redb`
- disposable media cache: `ProjectDirs::cache_dir()/media`
- config: `ProjectDirs::config_dir()`

The durable database persists both directions:

- forward: `post_id → PostRecord`
- reverse: `tag → roaring(post_id)`
- sort lanes: score and favorite indexes, with post-id ordering as the newest lane

## Query Path

Warm-cache filtering is a bitmap intersection over persisted `roaring` sets. Sorting is either an ordered lane walk for broad sets or a bounded local candidate sort for smaller intersections. UI query changes do not hit the network.

Danbooru HTTP is only an anonymous read-only ingress. The worker calls `GET /posts.json`; no login, API key, write endpoint, vote endpoint, or mutation primitive exists in the code.

## Multi-Tag Reality

Danbooru is the ingestion oracle, but not the interaction engine. Live anonymous search can be tag-count constrained; local search is not. The current warmer sends at most two positive tags plus an order metatag, absorbs the resulting posts, and then the local index enforces the full query. Later crawlers should choose the two rarest locally-known tags as the remote seed.

## Pure Rust UI

The UI is `egui`/`eframe`, backed by native `winit`/`wgpu`. There is no JavaScript surface. Background threads perform network and decode work, then send decoded RGBA blades to the UI thread for texture upload.

## Future Boorus

The seam is `Booru::posts(Query, Sort, page) -> Vec<PostRecord>`. Other boorus should map their wire format into the canonical `PostRecord`, then reuse the same index, sort lanes, media cache, and UI.
