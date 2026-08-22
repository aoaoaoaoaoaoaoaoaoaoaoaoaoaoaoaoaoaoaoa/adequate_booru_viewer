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

optional post-tag editing uses a Danbooru login and an API key stored in a
separate file. On Linux, add this to
`~/.config/adequate_booru_viewer/config.toml`:

```toml
[danbooru.account]
login = "example_user"
api_key_file = "/home/example/security/danbooru.token"
```

The file contains only the API key. ABV loads it at startup; the first push
begins with a safe authenticated read and reports any rejected credential or
permission. The anonymous mirror never receives the credential. Relative key
paths resolve beside `config.toml`.

Saved filters use a small Boolean language rather than exposing the editor's
tree representation:

```toml
[[filter]]
name = "general rating"
query = "rating:g"

[[folder]]
name = "eyes"

[[folder.filter]]
name = "visible blue eyes"
query = "blue_eyes AND ~(closed_eyes OR /(^|_)covered_eyes$/)"
```

`~`, `AND`, `XOR`, and `OR` bind in that order; parentheses override
precedence. Operators are uppercase, while tags and `rating:*` atoms are
lowercase. Regexp atoms use `/pattern/`. An empty query matches everything.
`XOR` retains ABV's selection-group meaning: exactly one operand must match.
Older generated filter trees migrate automatically; unrelated formatting and
comments remain untouched.

the release-tested native coordinates are Linux/X11, Linux/Wayland, macOS on
Apple and Intel silicon, and 64-bit Windows. `abv --pause-mirror` starts with
the mirror valve closed for deterministic or disconnected work.

### keyboard

press `F1` or `?` for the generated command guide. `Tab` and `Shift+Tab` move
within the active inspector panel; physical `Control+Tab` and
`Control+Shift+Tab` cross panels. `Alt+F` focuses the tag entry, `Alt+G` selects
the next Boolean group, and `Alt+Shift+G` selects the previous one. Completion
suggestions retain local `Tab` and `Shift+Tab` while the tag entry owns them.

`F9` conceals or reveals the Inspector. In the gallery, `PageUp` and `PageDown`
move exactly one row and `Home` returns to the first row. The full viewer uses
`T` for its tag drawer, `R` for the family tree, Left and Right for global
results, Alt+Left and Alt+Right for family peers, and Up and Down for the parent
or first child. Inside the tree, plain arrows navigate the family and Alt+Left
or Alt+Right returns to global-result navigation. `R`, Enter, or Escape returns
to the selected image; Escape from the image closes the viewer. The guide is
modal: application commands and panel traversal remain dormant until it
closes, then the prior focus target is restored.

`F2` opens application settings; `Control+,` does the same on Linux and
Windows, while `Command+,` is the macOS form. Hover prefetch and the background
mirror share this central sheet with their contextual controls. The sheet names
the XDG configuration file, reports invalid or unknown TOML keys without
overwriting their source, and offers an explicit reload after repair.

### architecture

ABV owns booru semantics, indexing, workers, configuration, its Filter Library,
typed command consequences, and its gallery and viewer. `eternalist-apps` supplies the
one-window native lifecycle and the logical Inspector, Cabinet, LivingWait,
CommandGuide, CommandCanon, and PanelNavigator assemblies. Brass Poolrooms
owns the physical controls, material language, and living water. The
saved-filter active card and immutable local-favorites row remain
product-specific; the reorderable, one-level foldered filter collection uses
the shared Cabinet law.

The Filter Library is user-owned product data under XDG data; configuration
contains only small human-edited settings, and Session State contains disposable
workbench ephemera. Persistence settlement and the crawl refresh gates are semantic service
deadlines, independent of rendering. Strict, format-preserving configuration
crosses the Eternalist configuration ledger; disposable workbench state crosses
its own background scribe. Visual fades, recoils, and hover dwell alone retain
repaint timing.

Native acceptance lives here because its fixtures, semantic targets, and
verdicts are product behavior. `scripts/test-gui` first proves an ordinary
uninstrumented launch, then drives the optimized witnessed binary in private
X11, XDG, process, network, and software-graphics namespaces. It proves the
seeded filter, generated help pixels, modal containment, keyboard-only panel
and query movement, focused rail adjustment, rendered dry-to-wet transition,
independent Filter Library and Session State recovery, water and open-viewer
restoration across restart, explicit viewer dismissal, native clipboard copy,
and browser dispatch. `scripts/test-wayland` owns the narrower native launch,
first-present, typed-witness, and nonblack compositor-capture gate.

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
judging the complete evidence graph and finding the exact crate version on
crates.io.

anyway, check out how wet it is:

[![the wet demo](https://raw.githubusercontent.com/aoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoa/adequate_booru_viewer/v1.0.0/docs/abv-wet-teaser.webp)](https://github.com/aoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoaoa/adequate_booru_viewer/releases/download/v1.0.0/abv-wet-demo.mp4)

*(click through for the full 60-second take)*

### halp it's missing feature XYZ

tell your fable to make a good pr and I'll tell mine to consider it

no promises
