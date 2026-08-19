# Lock-screen glass: the dev loop

Three harnesses, none of which touch the live session.

| | |
|---|---|
| `./lock-glass-shot.sh out/` | one frame per backend, GL and GSK, for before/after |
| `LIVE=1 ./lock-glass-shot.sh out/` | two frames a second apart, over a moving backdrop |
| `./lock-glass-sweep.sh out/ 'label\|FROST=3,FRESNEL=0.9' ...` | a labelled sheet, one tile per settings combination |
| `glass-demo/lockprobe.sh` | does GL work on a real `ext-session-lock-v1` surface |

All of them run `swaypplet --preview lock` inside a **nested headless sway**.
That is not paranoia about the lock screen — `--preview lock` builds a plain
toplevel and never takes a session lock. It is because a session whose outputs
are asleep (`swaymsg -t get_outputs` showing `power: false`) sends no frame
callbacks at all: GTK's frame clock stalls after two frames and `grim` blocks
on screencopy. The nested compositor drives frames regardless.

`lockprobe.sh` is the exception that does take a lock, and it is guarded three
ways because `ext-session-lock-v1` deliberately leaves a session locked when
its locker dies. See the module docs in `glass-demo/src/lockprobe.rs`.

## Tuning

Every field of `Tuning` in `src/lock/glass_gl.rs` reads an environment variable
named after it, so nothing here needs a rebuild:

```sh
SWAYPPLET_GLASS_FROST=0 SWAYPPLET_GLASS_FRESNEL=1.0 swaypplet --preview lock
```

The knobs worth reaching for first:

- `FROST` — how blurred the refraction is. This is a lens; blur is what fights it.
- `FRESNEL` / `REFLECT_LOD` — how strong and how sharp the rim reflection is.
- `ABSORB` — how smoked. One scalar; the channel ratios that make the tint are fixed.
- `THICKNESS` / `BEZEL` — the geometry, and the biggest lever by far.
- `RADIUS` — only useful alongside a matching `.lock-card` border-radius.

`glass-testcard.png` is the default backdrop for the sweep on purpose:
refraction and reflection are both displacements, and a displacement is
invisible on content with no structure to displace.
