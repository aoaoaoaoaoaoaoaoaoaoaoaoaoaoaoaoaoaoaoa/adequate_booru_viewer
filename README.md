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

linux (rust 1.96+):

```sh
cargo install adequate_booru_viewer   # gives you the `abv` binary
```

you also want a Vulkan driver and either X11 or Wayland libraries (your
distro's Mesa/Vulkan ICD and libxkbcommon).

releases also carry an unsigned universal macOS disk image and an unsigned
64-bit current-user Windows installer. Gatekeeper or SmartScreen may require an
explicit first-launch override.

first launch starts an anonymous, read-only, persistent danbooru mirror which may
grow to tens of gibibytes. pause it under `INDEX STATUS`; closing `abv` stops it.
media bytes remain disposable cache.

the release-tested native coordinates are Linux/X11, Linux/Wayland, macOS on
Apple and Intel silicon, and 64-bit Windows. `abv --pause-mirror` starts with
the mirror valve closed for deterministic or disconnected work.

### architecture

ABV owns booru semantics, indexing, workers, configuration, and its gallery and
viewer. `eternalist-apps` supplies the one-window native lifecycle and the
logical Inspector, Cabinet, and LivingWait assemblies. Dwemer Poolrooms owns
the physical controls, material language, and living water. The saved-filter
active card and immutable local-favorites row remain product-specific; the
reorderable, one-level shelved filter collection uses the shared Cabinet law.

Native acceptance lives here because its fixtures, semantic targets, and
verdicts are product behavior. `scripts/test-gui` first proves an ordinary
uninstrumented launch, then drives the optimized witnessed binary in private
X11, XDG, process, network, and software-graphics namespaces. It proves the
seeded filter, rendered dry-to-wet transition, durable slate update, restart
restoration, and return to dry. `scripts/test-wayland` owns the narrower native
launch, first-present, typed-witness, and nonblack compositor-capture gate.

For local release-candidate work:

```sh
scripts/check
scripts/audit
scripts/verify-install
scripts/test-gui
scripts/test-wayland
scripts/package
```

the pinned Eternalist Foundry workflow proves every declared host, builds and
tests the unsigned DMG and NSIS installer, and publishes artifacts only after
judging the complete evidence graph.

anyway, check out how wet it is:

[![the wet demo](https://raw.githubusercontent.com/aoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoa/adequate_booru_viewer/v1.0.0/docs/abv-wet-teaser.webp)](https://github.com/aoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoa/adequate_booru_viewer/releases/download/v1.0.0/abv-wet-demo.mp4)

*(click through for the full 60-second take)*

### halp it's missing feature XYZ

tell your fable to make a good pr and I'll tell mine to consider it

no promises
