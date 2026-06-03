# Booru Bayonet Design

Goal: a native, anonymous, read-only Danbooru reference workbench whose warm-cache interaction path is local index math, not live HTTP search.

## Storage Contract

The objection that long scraping work must survive restart is correct. The app treats index and media differently:

- durable index: `ProjectDirs::data_local_dir()/index.redb`
- disposable media cache: `ProjectDirs::cache_dir()/media`
- config snapshot: `ProjectDirs::config_dir()/config.toml`

The durable database persists both directions:

- forward: `post_id → PostRecord`
- reverse: `tag → roaring(post_id)`
- rating lane: `rating → roaring(post_id)`
- sort lanes: score and favorite indexes, with post-id ordering as the newest lane
- semantic lane: `post_id → normalized jina-clip-v1 image embedding`
- crawl cursor: latest Danbooru passive-crawl `page=b<post_id>` frontier

Posts tagged `animated` are outside the reference-workbench contract. The ingestion path refuses to insert them, and background maintenance purges already-cached animated posts from the forward table, tag/rating/sort lanes, and CLIP embedding lane.

`config.toml` is a serde/TOML snapshot of active state: `[query]` include/exclude chips, `[view]` sort and tile scale, and `[soft]` CLIP prompt/α. Startup restores it before the first local search.

Model weights live under `ProjectDirs::data_local_dir()/models`, not under the disposable media cache. Pulling ONNX weights is work; it should survive restart.

## Query Path

Warm-cache filtering is bitmap algebra over persisted `roaring` sets: positive tags intersect, rating metatags union within their lane, and negative tags/ratings subtract. Sorting is either an ordered lane walk for broad sets or a bounded local candidate sort for smaller intersections. UI query changes do not hit the network.

Soft CLIP sort is a rerank over a local candidate pool: `base_rank + α * cosine(text_embedding, image_embedding)`, with the base score normalized before mixing. Missing image embeddings are queued lazily and persisted once computed.

Danbooru HTTP is only an anonymous read-only ingress. The worker calls `GET /posts.json`; no login, API key, write endpoint, vote endpoint, or mutation primitive exists in the code.

A passive crawler walks Danbooru newest-to-oldest with `page=b<id>` and a durable cursor. The active query warmer separately walks page 1, 2, 3, ... for the current query/sort until exhaustion or Danbooru's anonymous 1000-page search cap, then re-runs local search as pages are absorbed, so score/favorite sorts keep widening while the cache warms. Both paths share one 150 ms read gate, about 6.7 requests/sec against Danbooru's documented 10 requests/sec read ceiling.

The UI reports cache status directly: indexed posts, tag keys, stored CLIP image embeddings, per-rating counts, newest known post, and the passive crawl frontier. The displayed crawl percentage is an ID-space estimate, not a claim about undeleted corpus cardinality.

## Multi-Tag Reality

Danbooru is the ingestion oracle, but not the interaction engine. Live anonymous search can be tag-count constrained; local search is not. The current warmer sends at most two positive tags plus an order metatag, absorbs the resulting posts, and then the local index enforces the full query. Later crawlers should choose the two rarest locally-known tags as the remote seed.

## Pure Rust UI

The UI is `egui`/`eframe`, backed by native `winit`/`wgpu`. There is no JavaScript surface. Background threads perform network and decode work, then send decoded RGBA blades to the UI thread for texture upload.

The main grid is image-only for scan speed. Filter state lives in a left chip panel. Thumbnails expose tag mutation on hover (`-` banishes, `+` requires, `×` removes). Clicking a thumbnail opens a scaled full-image frame with copy and right-click-close.

`Ctrl` + mouse-wheel scales the grid from half-size to triple-size. Danbooru currently exposes media variants named `180x180`, `360x360`, `720x720`, `sample`, and `original`; the viewer stores the 180/360/720 URLs when present and chooses the thumbnail bucket from the current tile edge. The full-image frame uses sample/original fallbacks.

## Future Boorus

The seam is `Booru::posts(Query, Sort, page) -> Vec<PostRecord>`. Other boorus should map their wire format into the canonical `PostRecord`, then reuse the same index, sort lanes, media cache, and UI.
