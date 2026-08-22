# adequate booru viewer — design

Goal: a native Danbooru reference workbench whose warm-cache interaction path is local index math, not live HTTP search. Its mirror is anonymous and read-only; a separately authenticated, explicitly configured lane may perform narrow user-requested edits.

## Storage Contract

The objection that long scraping work must survive restart is correct. The app treats index and media differently:

- durable index: `ProjectDirs::data_local_dir()/index.redb`
- disposable media cache: `ProjectDirs::cache_dir()/media`
- configuration: `ProjectDirs::config_dir()/config.toml` — small human-edited
  settings such as hover prefetch, mirror policy, and an optional account plus
  external API-key path
- Filter Library: `ProjectDirs::data_local_dir()/filters.toml` — saved filters
  and folders; user-owned product data, written independently and atomically
- Session State: `ProjectDirs::state_dir()/slate.toml` (data dir fallback
  off Linux) — scratch query, active filter, sort, density, folder collapse,
  and the open viewer's post/anchor/tree identity; the app's snapshot of
  itself, free to decay to defaults

The durable database persists both directions:

- forward: `post_id → PostRecord`
- reverse: `tag → roaring(post_id)`
- rating lane: `rating → roaring(post_id)`
- sort lanes: score and favorite indexes, with post-id ordering as the newest lane
- crawl cursor: latest Danbooru passive-crawl `page=b<post_id>` frontier

Posts tagged `animated` are outside the reference-workbench contract, as are posts the API serves with every media URL stripped (gold-walled or banned). The ingestion path refuses to insert either, re-absorption purges already-cached offenders from the forward table and tag/rating/sort lanes, and search hydration skips any media-less stragglers in the meantime.

Startup restores all three domains before the first local search. The split is
a contract: losing Session State must never lose user intent, so its loader
decays silently to defaults; invalid configuration and a damaged Filter Library
fail loudly. Upgrading from the former combined `config.toml` writes the Filter
Library first, then rewrites configuration, so a crash cannot strand saved
filters between stores. An open viewer is rebuilt from post ids against the
canonical index; records, family projections, textures, navigation seams,
predictor history, and viewport geometry never enter Session State.

## Query Path

Warm-cache filtering is recursive bitmap algebra over persisted `roaring` sets. The runtime query is a tree of tag, regexp, and `rating:*` atoms plus `AND`, `OR`, `XOR`/select, and unary `NOT`; textual `-tag` is only entry-field sugar for `NOT tag`. `AND` intersects, `OR` unions, `XOR` keeps posts present in exactly one child, and `NOT` subtracts from the cached post universe. Saved filters cross the Filter Library boundary as canonical Boolean expressions with `~ > AND > XOR > OR` precedence; editor paths and group-selection state never enter the library. Loading migrates the former serialized-tree schema atomically. Sorting is either an ordered lane walk for broad sets or a bounded local candidate sort for smaller intersections. UI query changes do not hit the network.

Danbooru indexing remains anonymous read-only ingress. The crawler and warmer
call `GET /posts.json` through one credential-free client. An optional account
configuration names a login and an external API-key file; startup validates it
locally, then a separate single-consumer lane may add an existing tag to one
post. That lane authenticates by fetching the current post, submits its full
tag string plus `old_tag_string`, and absorbs the authoritative response. A
failed mutation is refetched before its outcome is reported. API-key bytes
never enter configuration, state, URLs, logs, or the mirror client.

A passive crawler walks Danbooru newest-to-oldest with `page=b<id>` and a durable cursor. The active query warmer separately walks page 1, 2, 3, ... for the current query/sort until exhaustion or Danbooru's anonymous 1000-page search cap, then re-runs local search as pages are absorbed, so score/favorite sorts keep widening while the cache warms. Both paths share one 150 ms read gate, about 6.7 requests/sec against Danbooru's documented 10 requests/sec read ceiling.

The UI reports cache status directly: indexed posts, tag keys, per-rating counts, newest known post, and the passive crawl frontier. The displayed crawl percentage is an ID-space estimate, not a claim about undeleted corpus cardinality.

## Multi-Tag Reality

Danbooru is the ingestion oracle, but not the interaction engine. Live anonymous search can be tag-count constrained; local search is not. The current warmer extracts anonymous-safe positive atoms from the boolean tree, sends at most one rating atom and enough positive tag atoms to stay under Danbooru's practical tag budget plus an order metatag, absorbs the resulting posts, and then the local index enforces the full tree. Later crawlers should choose the rarest locally-known positive atoms as the remote seed.

## Pure Rust UI

The UI is `egui` on Eternalist Apps' native `winit`/`wgpu` lifecycle. Worker
events wake the native event loop directly. When the full-image viewer opens,
the UI renders to an offscreen texture, the Brass Poolrooms compositor runs its
dual-Kawase veil and persistent water field, and an SDF-masked composite keeps
the viewer window sharp. With water disabled, egui rasterizes straight into
the swapchain at zero added cost. There is no JavaScript surface. Background
threads perform network and decode work, then send decoded RGBA images to the
UI thread for texture upload.

The main grid is image-only for scan speed. Filter state lives in the
Inspector's boolean-tree panel. One group is active; tag entry, autocomplete,
and hover-palette mutations target that group. Group frames are color coded,
selectable, nest arbitrarily, and expose `AND`/`OR`/`XOR`/`NOT` controls.
Thumbnails expose tag mutation on hover (`-` inserts `NOT tag`, `+` inserts
`tag`, `×` removes existing occurrences). Clicking a thumbnail opens a scaled
full-image frame with copy and right-click-close.

`Ctrl` + mouse-wheel scales the grid from half-size to triple-size. Danbooru currently exposes media variants named `180x180`, `360x360`, `720x720`, `sample`, and `original`; the viewer stores the 180/360/720 URLs when present and chooses the thumbnail bucket from the current tile edge. The full-image frame uses sample/original fallbacks.

## Future Boorus

The seam is `Booru::posts(Query, Sort, page) -> Vec<PostRecord>`. Other boorus should map their wire format into the canonical `PostRecord`, then reuse the same index, sort lanes, media cache, and UI.
