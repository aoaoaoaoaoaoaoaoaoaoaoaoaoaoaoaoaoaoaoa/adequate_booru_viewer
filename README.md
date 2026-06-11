# adequate booru viewer

a booru viewer that is somewhat adequate

- it's fast. local bitmap math, microseconds, no spinner ever
- rich tag algebra. real boolean trees — and, or, xor, not, nested
- save your filters. name them, folder them, keep them

and what else do you need

## run

```shell
cargo run --release
```

type tags on the left, look at pictures on the right. right-click a thumb to
slice its tags. click for the full image. that's the whole thing.

anonymous, read-only, danbooru only. pure rust, one binary, no electron, no
javascript, nothing to install first.

## the boring parts

native egui on a hand-rolled winit/wgpu loop (no eframe), so it can do its own
shaders — the whole UI floats on water. hover a thumbnail and it surfaces;
ripples radiate, reflect off the walls, and lap at the sidebar. you didn't ask
for any of this. roaring-bitmap index in redb, warmed by a background crawler.
config and saved filters in plain toml you can edit by hand.

linux only in practice. written cross-platform, but that's a theory, not a
promise.
