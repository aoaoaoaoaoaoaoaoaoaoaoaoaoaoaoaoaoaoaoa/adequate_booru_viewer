# Brass Poolrooms

A design language for native Rust UI. Not yet hatched — adopted piecemeal, one
widget at a time, until it can stand on its own. Where this document speaks, it
is normative; where it is silent, defer to taste that would not embarrass it.

The date transport (`app/date_spool.rs`) is the reference implementation. When
in doubt, look there and ask what it would do.

## The room

A vast poolroom, deep underground, beyond space and time. The builders left
mid-stride and did not come back. Centuries on, nothing has decayed: the water
is still clean and laps gently, the brass still indexes on its schedule, the
floors are still square. **Derelict, not decrepit** — the absence is of people,
not of order.

The walls are brown, faded-amber tilework. Set into them are brass mechanisms —
springs, levers, rollers, dials, square buttons — that rise out of the wall when
summoned and sink flush when done. The metal is pushed harder than any real
material: more reflective, more *metal*, than midcentury brass ever was. But the
forms are high-modernist — flats, right angles, no filigree. Metallic finish,
modernist form. This is the line that keeps it out of steampunk: steampunk
ornaments its brass; we do not. Every visible element is structure or it is
gone.

## Laws

1. **Derelict, not decrepit.** Empty and eternal, never worn. No rust, no grime,
   no distress textures, no "aged" ornament. Emptiness is the mood; decay is
   forbidden.

2. **Modernist form, metallic finish.** Push the metal far past real materials.
   Keep the geometry austere. Brass ramps and warm speculars on shapes a Braun
   designer would sign off on. If it reads as steampunk, a form has grown an
   ornament — cut it.

3. **Flats, no filigree.** Nothing exists to decorate. Gear teeth, rivets,
   curls, bevels-for-their-own-sake: all banned. *Perfection is achieved when
   there is nothing left to take away.* Every line earns its keep mechanically.

4. **The machines emerge.** A control is not painted onto a surface; it is a
   mechanism that lives in the wall. Dormant, it sits flush. Woken, it surfaces
   on a spring. Idle ⇒ flush, active ⇒ raised, and the transition is physical.
   (See `lift_spring` in the date transport.)

5. **One pool; everything displaces it.** There is a single body of water under
   the whole UI (owned by `brass_poolrooms::water`). Any mechanism that moves shoves it —
   a roller drags a wake, a lever throws a basin, a tileset swap thumps the pool
   from beneath. A control that moves and leaves the water flat is a bug.

6. **Spring, not tween.** Motion is the relaxation of a spring–damper with mass,
   not an eased keyframe. Overshoot is welcome; things settle, they do not snap
   to a stop. Fades and alpha ramps are a fallback, used only where a spring
   would be absurd.

7. **Physical first, but not procrustean.** Honor the mechanism — until honoring
   it would force the *content* into a bed it does not fit. Then concede
   gracefully and document the concession. A rigid slide-rule carriage with
   uniform detents is the physical ideal; an elastic carriage that breathes to
   each label's width is the concession. Both are legal. Default to the physical
   one and earn the exception.

## Material

The canonical values live in code; this points at them rather than copying them
(copies rot).

- **Tilework & ground** — `brass_poolrooms::chrome`: `PAGE`, `SURFACE`, `CONTROL` (the dark
  amber wall); `RAISED` (a woken plate); `EDGE` / `EDGE_STRONG` (lit seams).
- **The brass ramp** — `date_spool.rs`: `BRONZE_LO` → `BRONZE_MD` → `BRONZE_HI`
  (shadowed body → lit edge), sampled by `bronze(t)`. `RECESS_EDGE` borders a
  recess; `WELL` / `GUTTER` are the lightless voids a mechanism rises out of.
- **Pool tile** — `TILE_BASE` + per-tile `TILE_LIFT` / `TILE_CAST` scatter over
  a `GROUT` seam. Tiles vary; they are not a flat fill.
- **Heat** — `HOT` is the one warm amber that marks selection, focus, the live
  value. `TEXT` / `MUTED` are rest. Heat is scarce; spend it on exactly one
  thing at a time.
- **Light** — one warm key from above-and-toward-the-eye (`L_Y`, `L_Z`), a tight
  spec (`SHINE`, `GLOSS`), diffuse fill (`DIFFUSE`), and an ambient floor
  (`AMBIENT`) that keeps the deepest curl off pure black.
- **Motion** — spring constants are paired stiffness/damping tuned for a lively
  ζ≈0.4–0.5 overshoot: `SPRING_K`/`SPRING_C` (tape), `LIFT_K`/`LIFT_C` (emerge).

## The mechanisms

The growing vocabulary. Each is a brass form with a dormant and a woken state,
coupled to the pool.

| mechanism        | role                                  | status   |
|------------------|---------------------------------------|----------|
| The Pool         | universal medium; everything wakes it | built    |
| Spool / Reel     | scrolling tape transport (the date control) | built |
| Roller           | no-slip pulley a tape or carriage rides | built  |
| Lever            | arm / clear a bound; throws a basin wake | built  |
| Plate / Button   | square key; rings down after contact  | built    |
| Carriage selector| slide-rule cursor on a rail; 1-of-N   | building |
| Dial             | rotary scalar                         | planned  |

## Not this

- Steampunk: ornamental brass, exposed gears as garnish, Victoriana.
- Material Design flatness: shadows as a lie, ink ripples, no real light.
- Skeuomorphic clutter: stitching, leather, glass glare, faux screws.
- Badges, gradients-as-garnish, emoji chrome, anything that decorates instead of
  mechanizes.
- Decay: grunge, rust, scratches, "weathering." The room is clean.
