# glass-demo

True screen-space refraction in a GTK4 surface, with a custom GLSL shader.

This is the runnable answer to the open question in
[`docs/LIQUID_GLASS_RESEARCH.md`](../../docs/LIQUID_GLASS_RESEARCH.md): can a
swaypplet surface do real lensing, or only blur? It can. The demo is standalone
(its own crate, not built into `swaypplet`) and changes nothing in the shipped
binary.

```sh
nix develop ..              # from the repo root: nix develop
cargo build
../../target/debug/glass-demo ~/Pictures/wallpapers/pluto-4k.jpg
```

Without an image argument it reads `SWAYPPLET_LOCK_WALLPAPER`, and without
either it still runs on the test card alone.

## Controls

| | |
|---|---|
| drag | move a shape |
| click empty space | ripple: a travelling deformation of the height field, so the refraction moves with it |
| scroll over a shape | resize |
| `1`–`6` | presets: Regular, Clear, Prism, Liquid, Concave, Bar chips |
| `d` | debug views: off / normals / height field / merged SDF / light concentration |
| `t` | backdrop: mixed, wallpaper only, test card only |
| `n` | add a shape under the pointer (up to 8) |
| `r` | reset the layout |
| `p` | pause the backdrop animation |
| `Tab` | parameter panel |
| `q`, `Escape` | quit |

Moving the pointer steers the light direction, which is this demo's stand-in
for the gyroscope Apple drives the highlight with.

## What it actually does

`GskGLShader` was deprecated in GTK 4.16 and does not render under the current
renderers, which is usually where the "no custom shaders in GTK4" story stops.
`GtkGLArea` is still public and still hands you a raw GL context, so a client
can run arbitrary GLSL over content it owns.

Two passes, in [`shaders/`](shaders):

1. **`backdrop.frag`** draws the wallpaper plus a drifting test card into an
   offscreen colour buffer, and a mip chain is built over it.
2. **`glass.frag`** draws the glass. The only thing it can read is that buffer.

The offscreen pass is not incidental. A glass effect drawn straight over a
static image can fake refraction by warping that one image; here the backdrop
moves every frame and the glass pass has no access to the shapes' own colours,
which is the same shape as a compositor sampling its own composited output.

Per pixel, everything derives from one merged signed distance field:

```
merged SDF ──▶ height field ──▶ normal ──▶ Snell refraction ──▶ spectral samples
                            └─▶ Laplacian ─▶ light concentration
```

- **Merged SDF.** Shapes are unioned with a polynomial smooth minimum, so two
  that approach each other fuse through a continuous neck. Because every
  optical term below reads this one field, the fused blob refracts as a single
  body of glass rather than two overlapping ones. This is the `Liquid` preset,
  and it is the part none of the Linux prior art does.
- **Height field.** Four surface profiles (convex circle, convex squircle,
  concave, lip), the same four kube.io and scenefx-enhanced converged on, kept
  under the same names so the results are comparable.
- **Normal and Laplacian** come from the same five taps of the height field,
  by central difference. Not an analytic derivative, because the field also
  carries the smooth-union necks and the ripples.
- **Snell refraction.** `refract()` with a real index, then the ray descends
  the height it entered at to reach the backdrop plane; that descent is the
  entire source of the lateral offset. A flat top gives zero offset, which is
  why only the bezel lenses, and the sample point moves *inward*, which is why
  the glass magnifies. Both fall out of the physics rather than being dialled
  in.
- **Spectral dispersion.** Not the usual three-tap RGB fringe: N wavelengths
  across 400–700 nm each get their own index from Cauchy's equation and their
  own backdrop sample, recombined through Wyman, Sloan & Shirley's Gaussian fit
  to the CIE 1931 colour matching functions. That is what makes the `Prism`
  preset throw a real prism edge. Tap count is a slider, 1 to 24.
- **Light concentration.** The Laplacian of the height field is the thin-lens
  convergence term, and it is free — the five normal taps already paid for it.
  Convex-down shoulders focus and brighten, convex-up rims diverge and darken,
  which is Apple's "concentrates light" claim without a separate pass.
- Plus Schlick–Fresnel against a blurred offset copy of the backdrop as a
  stand-in environment, Blinn-Phong specular, a directional rim line, frost as
  a mip LOD that grows with path length through the glass, contact shadow, and
  grain.

## Cost

GPU time for the glass pass alone, measured in-shader with `GL_TIME_ELAPSED`
double-buffered queries (so nothing stalls), at 1600×1000 on an Intel Arc 140V:

| preset | spectral taps | glass pass |
|---|---|---|
| Regular | 4 | 1.12 ms |
| Clear | 8 | 1.12 ms |
| Bar chips | 3 | 1.21 ms |
| Liquid | 6 | 1.27 ms |
| Concave | 6 | 1.28 ms |
| Prism | 16 | 1.92 ms |

Tap count barely moves the number because the spectral loop only runs on
pixels inside the shape; everything outside early-outs after one SDF
evaluation. `Prism` costs more for its 78 px bezel, not its 16 taps. The HUD
shows this live, next to the wall-clock frame interval — read the GPU number,
since the wall-clock one measures the compositor's pacing as much as the
shader.

## Capture

```sh
./capture.sh out/                       # one PNG per preset
./capture.sh out/ wall.jpg 1920x1080 1  # test card only
GLASS_DEMO_DEBUG=1 ./capture.sh out/    # normals view
```

`capture.sh` runs the demo inside a nested headless sway and lets it read its
own framebuffer back with `glReadPixels`. That is deliberate: a session whose
outputs are asleep sends no frame callbacks, so GTK's frame clock stalls after
two frames and `grim` blocks on screencopy. The nested compositor drives frames
regardless, and nothing is captured from, or changed on, the real outputs.

`GLASS_DEMO_TESTCARD` (0–1) and `GLASS_DEMO_DEBUG` (0–4) set the starting
backdrop mix and debug view.

## Does it work on the lock surface?

Yes, verified:

```
$ ./lockprobe.sh out/
GtkGLArea renders on an ext-session-lock-v1 surface: 1600x1000,
glass pass 1.074 ms GPU, 4 frames drawn
```

`swaypplet --preview lock` renders the lock UI in a plain toplevel, which is
the right way to iterate on styling but proves nothing here: a session-lock
surface is created through a different protocol path, and whether GTK hands it
a GL context was the one thing standing between this demo and
`src/lock/glass.rs`. So `lockprobe.sh` takes a real lock, puts a `GtkGLArea`
on it, reads the framebuffer back, and unlocks. Same 1.07 ms as the toplevel.

It runs the lock **inside a nested headless sway**, never in the host session.
`ext-session-lock-v1` deliberately leaves a session locked when its locker
dies, so a probe that got this wrong would lock you out of your own desktop.
Three independent guards, detailed in [`src/lockprobe.rs`](src/lockprobe.rs):
an explicit `GLASS_DEMO_LOCK_PROBE=1`, a refusal when `WAYLAND_DISPLAY` is the
socket `lockprobe.sh` was started from, and an unconditional unlock timer armed
*before* `lock()` is called. It never touches PAM and never reads input.

## Where this applies in swaypplet

Unchanged from the research note: on Wayland a client cannot read the desktop
behind its own surface. This technique works wherever swaypplet already owns
the pixels underneath.

- **Lock screen — shipped.** `src/lock/glass_gl.rs` is the port: the same two
  passes, cut down to one rounded rect, with the GSK blur kept as a per-pane
  fallback. Tune it live with `SWAYPPLET_GLASS_*` and compare looks with
  `dev/lock-glass-sweep.sh`. It differs from this demo in four ways that only
  showed up against a real card — a captured margin so rim reflections have
  something outside the edge to find, a bezel clamped to the corner radius,
  the scrim applied after the optics rather than before, and Beer-Lambert
  absorption instead of a tint. See §Status of the research note.
- **Bar, panel, notifications — no.** Those are transparent layer-shell
  surfaces over a compositor-owned backdrop. Client-side refraction has nothing
  to sample. The `Bar chips` preset shows what it would look like, and what it
  costs, but shipping it means compositor-side work (see §3 of the research
  note).
