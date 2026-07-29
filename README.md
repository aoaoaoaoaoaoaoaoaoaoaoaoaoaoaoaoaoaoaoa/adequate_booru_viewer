## adequate booru viewer

a booru viewer that is somewhat adequate

1. it's very very very fast
2. tag boolean algebra. nest up to 8. powerful.

3. *it's wet!!!*

like really, really, soaking wet. drenched.

what else do you need?

maybe it has easter eggs if I wasn't too lazy.

and no, it's not an organizer.

### install

linux & macos (rust 1.96+):

```sh
cargo install adequate_booru_viewer   # gives you the `abv` binary
```

linux also wants a vulkan driver + X11/Wayland libs (your distro's mesa-vulkan /
vulkan-icd and libxkbcommon). macos uses metal, nothing extra.

first launch starts an anonymous, read-only, persistent danbooru mirror which may
grow to tens of gibibytes. pause it under `INDEX STATUS`; closing `abv` stops it.
media bytes remain disposable cache.

linux/X11 is the release-tested coordinate. wayland and macos are carried by the
same winit/wgpu/rfd stack and platform-neutral owned code, but not release-tested.

anyway, check out how wet it is:

[![the wet demo](https://raw.githubusercontent.com/aoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoa/adequate_booru_viewer/v1.0.0/docs/abv-wet-teaser.webp)](https://github.com/aoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoa/adequate_booru_viewer/releases/download/v1.0.0/abv-wet-demo.mp4)

*(click through for the full 60-second take)*

### halp it's missing feature XYZ

tell your fable to make a good pr and I'll tell mine to consider it

no promises
