# Motion

Every animated surface in swaypplet moves on one scale, with one curve family,
evaluated identically in Rust and in CSS. This is that scale. `src/anim.rs`
implements it, `data/style.css` declares it, and both point here.

## Why this needed doing

`src/anim.rs` has documented a three-duration scale since it was written, and
the Rust-driven surfaces obeyed it. The stylesheet drifted anyway, because
GTK4 CSS has no custom properties — `@define-color` is colours only — so every
duration is a bare literal and nothing stops a new one appearing. A census
before this change:

- **Durations:** 150 ms (×53, the intended micro tier), then 160, 180, 200,
  240, 280, 300, 320, 350, 380, 420, 550 and 900, most of them used once.
- **Easings:** `ease` (×63), `cubic-bezier(0.2, 0, 0, 1)` (×7), `ease-in-out`
  (×6), `ease-out` (×5), `linear`, and one bespoke shake curve.
- **Mechanism:** every modal enters through `anim::Reveal` — the launcher, the
  OSD, dmenu, the switcher, the keybinding sheet, the panel, the bar,
  notifications and the polkit dialog — **except the lock screen and the
  greeter**, whose card enters through a CSS keyframe instead.

And one thing that was simply backwards: `ease_out_cubic` drove every
Rust-side animation, exits included, so surfaces left the screen on a
decelerating curve. A thing leaving should accelerate away; decelerating into
absence reads as hesitation.

## The curves

Material 3's standard set, because seven declarations in the stylesheet
already used it, it is documented and defensible, and its enter/exit pairing
is the part swaypplet was missing.

| role | cubic-bezier | use for |
|---|---|---|
| **standard** | `cubic-bezier(0.2, 0, 0, 1)` | anything that moves between two on-screen states: reflow, colour, a value settling |
| **decelerate** | `cubic-bezier(0, 0, 0, 1)` | anything arriving — it starts at speed and settles |
| **accelerate** | `cubic-bezier(0.3, 0, 1, 1)` | anything leaving — it gathers speed and goes |

Rust evaluates these as real cubic-bezier curves (`anim::standard`,
`anim::decelerate`, `anim::accelerate`), not as an approximation, so a card
driven from Rust and a label driven from CSS move on the same curve. That is
the whole point of naming them.

Two curves stay outside the set, and both earn it:

- `@keyframes auth-shake` keeps `cubic-bezier(.36,.07,.19,.97)`. It is an
  oscillation, not a transition; an easing meant to get from A to B has
  nothing to say about a thing that returns to where it started.
- Progress spinners keep `linear`. A spinner that eases is a spinner that
  looks broken.

A surface that both fades and slides uses **one** curve for both, on one clock.
`anim::Reveal` drives its `SlideBin` from the same tick callback, off the same
start stamp and the same duration as the alpha, so the card is exactly as far
along its travel as it is along its fade in every frame. The settle used to run
on `standard` while the fade ran on `decelerate`/`accelerate`, which on the way
out is a decelerating motion against an accelerating fade: measured on a
40x-stretched exit, the panel finished 89 % of its 24 px by 61 % of the
duration, with the alpha still at 0.51, then stood still for the rest while the
alpha collapsed. That is what "leaves abruptly" looks like from the inside.
`standard` stays what the table says it is — the curve between two *on-screen*
states — and `SlideBin::slide_to`'s remaining callers (a card changing slots, a
drag springing back, `nudge`) are all of those.

## The durations

Five tiers. Every one is a Material 3 duration token, and the first three are
the scale `anim.rs` already had.

| tier | ms | for |
|---|---|---|
| **micro** | 150 | colour, opacity, a mark lighting up. Below conscious notice. |
| **exit** | 200 | anything leaving the screen |
| **enter** | 300 | anything arriving, and anything reflowing |
| **emphasis** | 400 | a one-shot that must be *noticed*: a verdict, an arrival, a refusal |
| **dwell** | 500 | a flourish that has to be readable while it plays |

Exits are shorter than entrances. Both Material and Apple say so and both are
right: waiting for something to leave is dead time, while an entrance is the
thing you are waiting for.

Ambient loops are not on this scale and should not be. A pulse, a breathing
glow, a spinner and a ring sweep are rhythms rather than transitions —
`auth-fp-pulse` at 2 s, `media-breathing` at 2 s, `face-ring-spin` at 1.1 s,
`rail-confirm-pulse` at 0.8 s. Their periods are chosen against a heartbeat,
not against a transition ladder, and snapping them to it would make them worse.

## What changes

| was | is | where |
|---|---|---|
| 160 ms | 150 | slot fades, auth marks |
| 180 ms + 50 delay | 200 + 50 | handoff content fade |
| 240 ms + 120 delay | 300 + 100 | handoff card fade |
| 280 ms | 300 | picked chip |
| 320 ms | 300 | handoff bloom |
| 350 ms | 300 | `face-ring-fail` |
| 380 ms | 400 | `auth-shake` |
| 420 ms | 400 | `polkit-icon-ok`, `face-pill-arrive` |
| 550 ms | 500 | `face-ring-ok`, `face-pill-ok` |
| 900 ms | 500 | `auth-field-reject` |
| `ease_out_cubic` on exits | `accelerate` | `anim::Reveal`, every surface |
| CSS keyframe entrance | the shared curve at the shared duration | lock and greeter card |

Two keyframes also stopped being made of shadow. `face-pill-attention` and
`face-pill-ok` were box-shadow glows, and under blur that shadow was the
reason the look-at-the-camera cue could not be frosted: `blur_ignore_transparent`
discarded only pixels at alpha exactly 0, so every translucent shadow pixel
frosted into a flat halo the size of the surface. The cue is the one surface
that appears over arbitrary content with no card behind it, and unfrosted it
reads as a sticker on the screen.

Liquid glass lifted that ban — a mask threshold replaces "alpha exactly 0", so
a shadow under `threshold - 0.12` is backdrop and is discarded, which is why
`.face-pill-dark` keeps its 0.28 glow. The two keyframes stayed on the border
and the fill anyway, on motion grounds: they are one property on one node,
where a shadow that grows from 0 re-rasterises a differently sized gaussian
every frame, and the border says the same thing in the vocabulary the armed
auth field already uses.

The lock screen's pill is no longer the exception either. Its glass comes from
the compositor too, through the reserved `session-lock` namespace, so it
decodes no wallpaper and runs no shader of its own.

The handoff's stagger survives. Its delays are the choreography — chips step
back before the card dissolves — so they are rescaled onto the ladder rather
than flattened to a single duration.

`FACE_SETTLE` in `src/lock/mod.rs` moves 350 → 300 with `face-ring-ok`. It was
sized to that keyframe deliberately: the ring does its visible work by the
overshoot at 60%, and 60% of 500 is 300.

## The lock screen is not an exception

It looks like one, because the compositor cross-fades the whole lock surface
(`src/lock/fade.rs`, `docs/LOCK_TRANSITION_WIP.md`) and the card must not
animate itself on top of that — its opacity would become CSS × surface and it
would visibly lag the wallpaper under it. That is why `.lock-crossfade
.lock-card { animation: none; }` exists and it stays.

But the fade *itself* runs on `ENTER_MS`/`EXIT_MS` from this scale, and when
the compositor cannot cross-fade — an unpatched compositor, a before-sleep
lock, a relaunch onto an abandoned lock — the card falls back to its own
entrance, which is now the same curve and duration as every other modal's.
Same for the greeter, which is a plain toplevel and always uses it.

So the rule holds on both paths: **one curve, one duration, whoever is
driving.**

## Keeping it

Nothing in GTK4 CSS can enforce this, so a check does. `dev/motion-census.sh`
prints every duration and easing in the stylesheet with its count; anything
off the ladder shows up as a one-line diff against the table above.

Reduced motion is handled once, in `anim::duration`, which collapses every
span to a single frame rather than to zero — the state flow that rides the
animation tick still has to run.
