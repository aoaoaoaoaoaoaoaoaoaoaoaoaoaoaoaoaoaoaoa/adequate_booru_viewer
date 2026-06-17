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

linux & macos (needs recent stable rust):

```sh
cargo install adequate_booru_viewer   # gives you the `abv` binary
```

linux also wants a vulkan driver + X11/Wayland libs (your distro's mesa-vulkan /
vulkan-icd and libxkbcommon). macos uses metal, nothing extra.

anyway, check out how wet it is:

<video src="https://github.com/aoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoa/adequate_booru_viewer/releases/download/v0.9.0/abv-wet-demo.mp4" controls muted width="100%"></video>

(no player? [grab the mp4](https://github.com/aoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoa/adequate_booru_viewer/releases/download/v0.9.0/abv-wet-demo.mp4))

### halp it's missing feature XYZ

tell your fable to make a good pr and I'll tell mine to consider it

no promises

### pretty video

make it yourself:

```sh
cargo xtask wet-demo
```
