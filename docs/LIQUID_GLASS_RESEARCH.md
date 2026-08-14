<!-- Research note, 2026-08-14. Nothing here is implemented. -->

# Liquid glass on swayfx + GTK4: findings

**Bottom line.** Real refraction is possible in both contexts, and both already have working prior art you can read. Compositor-side it is a solved problem with a shipping implementation for swayfx specifically (`swayfx-enhanced` + `scenefx-enhanced`), which handles layer-shell surfaces. Client-side, the picture changed in GTK 4.22: `GskGLShader` is dead, but GTK 4.22 shipped a `backdrop-filter` CSS property and a full SVG-filter pipeline including `feDisplacementMap` backed by a new `GSK_DISPLACEMENT_NODE`. That gives you displacement-map refraction with zero custom GL. The Wayland boundary still holds (a client cannot sample the desktop), but for the lock screen that boundary does not bite, because swaypplet already owns the wallpaper texture itself.

---

## 1. What Liquid Glass is, technically

Apple's own framing in [Meet Liquid Glass (WWDC25 session 219)](https://developer.apple.com/videos/play/wwdc2025/219/) names **lensing** as the defining mechanism: "Where as previous materials scattered light, this new set of materials dynamically bends, shapes, and concentrates light in real time." Beyond that it claims specular highlights that "respond to geometry", shadow opacity that adapts to what is behind ("increases the opacity of its shadow when it is over text"), thickness that scales with element size ("more pronounced lensing and refraction effects"), and gyroscope-driven highlight motion. Two variants: **Regular** (adaptive, legible over anything) and **Clear** (permanently transparent, needs a dimming layer).

Stripped of marketing, the distinct rendering techniques are four:

1. **Screen-space refraction.** Sample the backdrop at a UV offset derived from a surface normal computed from a height field over the shape's edge. This is the only genuinely new part versus a blur.
2. **Specular rim highlight.** `pow(dot(N, L), k)` against a fixed or motion-driven light direction.
3. **Blur of the backdrop** before or after refraction (frosting).
4. **Colour adaptation** (brightness/saturation/tint driven by backdrop luminance) and optional chromatic aberration.

The best technical breakdown is [kube.io, "Liquid Glass in the Browser: Refraction with CSS and SVG"](https://kube.io/blog/liquid-glass-css-svg/). It derives normals from a height function rather than a baked normal map (`normal = {x: -dz/dx, y: 1}` via central difference), applies Snell (n₁sinθ₁ = n₂sinθ₂, air = 1.0, glass = 1.5), and encodes the result into an `feDisplacementMap` RGBA image where `r = 128 + x*127, g = 128 + y*127` with 128 as neutral. It defines exactly four surface profiles: **convex circle** (`y = √(1-(1-x)²)`), **convex squircle** (`y = ⁴√(1-(1-x)⁴)`), **concave**, and **lip** (smootherstep blend). Note those four names, they reappear verbatim below. It explicitly skips chromatic aberration, multiple refraction bounces, and perspective, and it assumes incident rays orthogonal to a flat backdrop. Other reproductions: [Ken Sorrell's lens effect](https://www.sorrell.info/blog/liquid-glass-lens-effect), [CSS-Tricks](https://css-tricks.com/getting-clarity-on-apples-liquid-glass/), [ybouane/liquidglass](https://github.com/ybouane/liquidglass) (WebGL), [rdev/liquid-glass-react](https://github.com/rdev/liquid-glass-react).

---

## 2. GTK4 client-side: dead end and open door

**GskGLShader is gone in practice.** [The GSK docs](https://docs.gtk.org/gsk4/class.GLShader.html) state it plainly: "This feature was deprecated in GTK 4.16 after the new rendering infrastructure introduced in 4.14 did not support it. The lack of Vulkan integration would have made it a very hard feature to support." The headers `gskglshader.h` and `gskglshadernode.h` still exist in the 4.22.4 tree on this machine (verified at `/nix/store/v468fask1gcc4ia2hw70iapx45gdb33y-gtk4-4.22.4-dev/include/gtk-4.0/gsk/`), but the node does not render under the ngl or vulkan renderers. Treat it as unavailable.

**GTK 4.22 added a real `backdrop-filter` property.** Verified in source (`gtk/gtkcssstylepropertyimpl.c:1430` registers `"backdrop-filter"` with the same `filter_value_parse` as `filter`). The implementation is `gtk/gtkwidget.c:12084-12151`:

```c
backdrop_filter_value = style->other->backdrop_filter;
has_backdrop_filter = !gtk_css_filter_value_is_none (backdrop_filter_value);
if (has_backdrop_filter)
  gtk_snapshot_push_copy (snapshot);
...
  gtk_snapshot_push_rounded_clip (snapshot, border_box);
  extra_size = gtk_css_filter_value_push_snapshot (backdrop_filter_value, snapshot);
  gtk_snapshot_append_paste (snapshot, &bounds, 0);
  gtk_css_filter_value_pop_snapshot (backdrop_filter_value, &bounds, snapshot);
```

That is `GSK_COPY_NODE` / `GSK_PASTE_NODE` (both new in 4.22): copy the render canvas as painted **so far in this toplevel's node tree**, re-paste it through the filter, clipped to the widget's border box. When the filter needs samples outside the widget bounds, GTK pads with `GSK_REPEAT_REFLECT` (an edge mirror), which is direct evidence it has nothing outside to reach for. The official demo is `demos/gtk-demo/transparent.c`, whose header comment is "Blur the background behind an overlay" and whose CSS is `backdrop-filter: blur(14px)` on buttons in a `GtkOverlay` over a `GtkPicture`.

**Custom SVG filters, including displacement, are reachable from CSS.** `gtk/gtkcssfiltervalue.c` has `GTK_CSS_FILTER_SVG` holding `{char *url; char *ref; GtkSvg *svg;}`; parsing at line 1000 (`gtk_css_parser_has_url`) through 1107, application at line 1230 via `gtk_svg_apply_filter()`. The full SVG filter primitive set is implemented in `gtk/gtksvg.c` (verified strings in `libgtk-4.so.1.2200.4`): `feBlend feColorMatrix feComponentTransfer feComposite feDiffuseLighting feDisplacementMap feDropShadow feFlood feGaussianBlur feImage feMorphology feOffset feSpecularLighting feTile feTurbulence`. `feDisplacementMap` maps to `gsk_displacement_node_new()` (`gtk/gtksvg.c:21373`), whose own doc comment says: "modeled after SVG's feDisplacementMap filter... `value = scale * (value - offset)` and clamping the resulting value to be between `-max` and `max`. Since: 4.22". The node type is private API (`gsk/gskdisplacementnodeprivate.h`), so C/Rust code cannot construct it directly, but CSS reaches it, and the testsuite confirms a `data:` URI works inline:

```css
a { filter: url("data:image/svg+xml,<svg><filter id='yay'></filter></svg>#yay"); }
```
(`testsuite/css/parser/filter-svg.css`)

Since `backdrop-filter` and `filter` share one parser, `backdrop-filter: url("data:image/svg+xml,…#glass")` is valid and gives you kube.io's exact technique with no external file. One caveat found in source: `gtk/gtkcssfiltervalue.c:1099` calls `gtk_svg_set_features(svg, GTK_SVG_SYSTEM_RESOURCES & GTK_SVG_EXTERNAL_RESOURCES)`. With values `1<<1` and `1<<2` that bitwise AND is 0, so CSS-loaded filter SVGs run with all features off. Harmless for a self-contained displacement filter; it does mean no external references and no SMIL animation inside the filter.

**The Wayland boundary, precisely.** A GTK4 client on Wayland cannot read the desktop content behind its own surface. There is no protocol that hands a client the backdrop, and `ext-background-effect-v1` was designed specifically to avoid it (see §4). So `backdrop-filter` on a layer-shell surface with a transparent background refracts nothing, because nothing was painted beneath it *in that surface*. What it can refract is anything the client painted below the widget in its own node tree.

**Which is exactly the lock screen's situation.** `/home/meros/git/personal/swaypplet/src/lock/glass.rs` already documents the workaround: the ext-session-lock surface paints the wallpaper itself as a crisp `Picture`, and `GlassPane` re-draws the same `gdk::Texture` blurred and scrim-dimmed, clipped to the card. The texture comes from `wallpaper_texture()` at `/home/meros/git/personal/swaypplet/src/lock/ui.rs:818`. Because the client holds the pixels, refraction of that wallpaper is unrestricted. Two routes:

- **CSS:** put `backdrop-filter: url("data:…#glass")` on `.lock-card` and drop the manual `GlassPane`. GTK copies the wallpaper `Picture` beneath and displaces it.
- **Manual snapshot:** keep `GlassPane`, but the displacement node is private API, so you would be limited to `push_blur` plus whatever you can express with public nodes. The CSS route is the one that gets you displacement.

For the bar, panel, notifications and polkit dialog (layer-shell, transparent, compositor-blurred, see `/home/meros/git/personal/swaypplet/src/anim.rs`), client-side refraction of the desktop is impossible. That has to be compositor-side.

---

## 3. Compositor-side: already implemented for swayfx

**scenefx's blur is dual-kawase**, confirmed by reading the shaders in `render/fx_renderer/shaders/`: `blur1.frag` is the downsample (5 taps, `uv = v_texcoord * 2.0`, `sum/8.0`), `blur2.frag` the upsample (8 taps, `sum/12.0`), plus `blur_effects.frag` for brightness/contrast/saturation/noise. Pass structure is `render/fx_renderer/fx_pass.c:1056-1066`, N downsamples then N upsamples. Defaults are `radius = 5, num_passes = 3, noise = 0.02, brightness = 0.9, contrast = 0.9, saturation = 1.1` (`types/fx/blur_data.c:3`).

**The refraction work already exists.** [CreitinGameplays/swayfx-enhanced](https://github.com/CreitinGameplays/swayfx-enhanced) ("Sway, but with Liquid Glass support", [site](https://creitingameplays.github.io/swayfx-enhanced/)) plus its renderer fork [CreitinGameplays/scenefx-enhanced](https://github.com/CreitinGameplays/scenefx-enhanced). Note the flake.nix in swayfx-enhanced still points at upstream `wlrfx/scenefx` (rev `05a5e7a`, 2026-01-14) which has no glass code, so a build needs redirecting at `scenefx-enhanced`. The shader is `render/fx_renderer/shaders/liquid_glass.frag`, 307 lines. Its structure:

- A soft-min SDF over a rounded rect with **two separate sharpness factors** (`sk_geo = 32.0` for clipping, `sk_grad = 6.0` for the normal), so corners stay square while the bevel normal stays smooth. The comment names the artefact it fixes: "softening the 'X' miter joint artifact".
- `get_surface_z_dz()` implements the same four profiles as kube.io, under the same names: `surface_type == 0` convex circle (`z = sin(x*π/2)`), `1` convex squircle (`z = 1.5x - 0.5x³`), `2` concave, `3` lip (`z = 0.5 - 0.5cos(πx)`).
- Refraction uses GLSL's built-in `refract(I, N, 1.0/refraction_index)` with `I = vec3(0,0,-1)`, then `offset = R.xy * (-h / |R.z|)` clamped to `±bezel_width`.
- Chromatic aberration runs `refract` three times at `1/(n±ca)` and samples R, G, B separately.
- Specular is `pow(max(dot(N, L), 0.0), 32.0) * specular_opacity` with `L = normalize(vec3(cos θ, sin θ, 1))`.
- Plus `brightness_boost`, `adjust_saturation()`, and hash noise.

**How it gets the backdrop** (`render/fx_renderer/fx_pass.c:1165-1256`) is the part a client can never replicate:

```c
int margin = (int)ceil(glass_data->bezel_width) + 1;
pixman_region32_init_rect(&capture_region,
    dst_box.x - margin, dst_box.y - margin,
    dst_box.width + 2 * margin, dst_box.height + 2 * margin);
fx_renderer_read_to_buffer(pass, &capture_region,
    pass->fx_effect_framebuffers->effects_buffer, pass->buffer);
```

It copies the already-composited output (plus a bezel-width margin so refracted samples stay inside captured pixels) into an effects buffer, then draws the glass quad sampling that copy. Public API is `fx_render_pass_add_liquid_glass()` in `include/scenefx/render/pass.h`, config struct in `include/scenefx/types/fx/liquid_glass_data.h`.

**It covers layer-shell.** `sway/desktop/layer_shell.c:108-143` creates a `liquid_glass_node` per layer surface, lowered to the bottom of the surface's tree (`:551`), so the client's own GTK content composites on top. It auto-disables when the surface's opaque region covers ≥95% of it. Per-layer control exists via `layer_criteria` (`include/sway/layer_criteria.h` has both `blur_enabled` and `liquid_glass_enabled`). That is a direct fit for swaypplet's bar/panel/notifications.

**It does not cover ext-session-lock.** `sway/lock.c` has no glass or blur node, matching the note already in `glass.rs`. The lock screen stays a client-side problem either way, which is fine given §2.

Config surface (from the README): `liquid_glass`, `liquid_glass_surface <convex_circle|convex_squircle|concave|lip>`, `liquid_glass_bezel_width`, `liquid_glass_thickness`, `liquid_glass_refraction_index`, `liquid_glass_chromatic_aberration`, `liquid_glass_specular`, `liquid_glass_specular_opacity`, `liquid_glass_specular_angle`, `liquid_glass_brightness_boost`, `liquid_glass_saturation_boost`, `liquid_glass_noise_intensity`. Marked Experimental; the roadmap's only item is "Improve Liquid Glass stability and performance." Recent commits are artefact fixes ("Fix liquid glass edge and seam artifacts", "address visual artifacts that appeared on tiny windows and squared shapes (ongoing)").

**Upstream scenefx has nothing.** No refraction code, no glass issues or PRs. The relevant open issues are [#60 "Comp-defined shaders"](https://github.com/wlrfx/scenefx/issues/60) (pre/post scene-node custom shaders, still open) and [#53 "Support user defined shaders"](https://github.com/wlrfx/scenefx/issues/53) (closed). [PR #204](https://github.com/wlrfx/scenefx/pulls) is an open "vulkan: implement blur", worth watching since a Vulkan renderer would change the porting surface.

---

## 4. Prior art on Linux

| Project | Where | Notes |
|---|---|---|
| [CreitinGameplays/swayfx-enhanced](https://github.com/CreitinGameplays/swayfx-enhanced) + [scenefx-enhanced](https://github.com/CreitinGameplays/scenefx-enhanced) | swayfx/scenefx | The direct match. Windows and layer surfaces. GLES2. Experimental. |
| [hyprnux/hyprglass](https://github.com/hyprnux/hyprglass) | Hyprland plugin | The most complete. Frosted blur, edge refraction, chromatic aberration, Schlick Fresnel, specular, inner shadow, centre dome lens distortion, adaptive tone mapping. `DECORATION_LAYER_BOTTOM` for windows; hooks `renderLayer` for layer surfaces (off by default, `layers = { enabled = true }`). ABI-pinned to Hyprland; the author warns layer support "may break across Hyprland updates". |
| [purple-lines/liquid-glass-plugin-hyprpm](https://github.com/purple-lines/liquid-glass-plugin-hyprpm) | Hyprland plugin | `blur_strength`, `refraction_strength`, `chromatic_aberration`, `fresnel_strength`, `specular_strength`. |
| [4v3ngR/kwin-effects-glass](https://github.com/4v3ngR/kwin-effects-glass) | KWin (Plasma 6.6/6.7) | Fork of the stock blur effect with **Snell's refraction**, force blur, rounded corners, tint/glow. Stock KWin blur is dual-kawase with no refraction. |
| [zaroutt/Niri-glass](https://github.com/zaroutt/Niri-glass) | niri fork | Shader at `src/render_helpers/shaders/clipped_surface.frag`, ported from kwin-effects-glass. Options: `refraction-strength`, `refraction-power`, `physical-refraction`, `lens-distortion`, `fringing`, `edge-lighting`, `glow-weight`, `adaptive-dim/boost`. Author's own caveat: "Vibe coded project so expect weirdly behavior." |
| [ryohsuke1231/liquid-glass](https://github.com/ryohsuke1231/liquid-glass) | GNOME Shell extension | Exists; I did not verify how much is refraction versus styling. |
| mutter / niri / stock blur | — | Blur only, via [`ext-background-effect-v1`](https://wayland.app/protocols/ext-background-effect-v1). |

**The protocol matters for planning.** `ext-background-effect-v1` (staging v1, by Xaver Hugl, merged to wayland-protocols, [Phoronix](https://www.phoronix.com/news/Wayland-Background-Effect)) has exactly one request that does anything, `set_blur_region`, and a capability enum with exactly one member, `blur = 1`. No refraction, no displacement, no way for the client to read the backdrop. [niri 26.04 implements it](https://www.phoronix.com/news/Niri-26.04-Released); mutter's implementation is [MR !5071](https://gitlab.gnome.org/GNOME/mutter/-/merge_requests/5071), which "captures the already-painted framebuffer contents behind the requested region, blurs them offscreen, and paints the blurred result back". So the standardised path will get you portable blur, never refraction. hyprglass has [issue #61](https://github.com/hyprnux/hyprglass/issues/61) asking to honour it.

---

## 5. Cost

I derived this from the actual shaders rather than trusting a benchmark blog, since I have exact tap counts. Let P = full-resolution pixels of the effect region.

**scenefx dual-kawase, defaults (N=3):**
- Downsample, 5 taps, writing at P/4 + P/16 + P/64 → **1.64P** taps
- Upsample, 8 taps, writing at P/16 + P/4 + P → **10.5P** taps
- `blur_effects` pass, 1 tap at P → **1P**
- Final composite through `tex.frag` → **1P**
- **≈14.1 taps/px, ~8 draw calls, 4 FBO swaps.** The damage region is first expanded by `blur_data_calc_size()` (roughly radius·2ᴺ per side) before any of this, so P exceeds the surface area.

**liquid_glass, card of w×h with bezel margin m:**
- Backdrop copy over (w+2m)(h+2m), 1 tap/px. For a 500×300 card with m=30 that is **1.34P**
- Glass shader, 1 tap/px, or 3 with chromatic aberration → **1P** or **3P**
- **≈2.3P taps (4.4P with CA), 2 draw calls, 1 FBO swap.**

So refraction moves **3–6× less texture bandwidth and issues ~4× fewer draw calls** than three-pass kawase. The tradeoff runs the other way on ALU: the glass shader's SDF alone costs eight `exp()` and three `log()` calls, plus `refract`, `normalize`, `pow(·,32)`. Call it ~60–100 ALU ops/px against kawase's ~10.

Concretely on this machine (Intel Arc 140V, Xe2, `xe` driver, LPDDR5X): a fullscreen 2880×1800 pass is 5.18 Mpx, so ~73 M texel fetches for kawase against ~12 M for glass; at 4K (8.29 Mpx) that is ~117 M against ~19 M. Both land under a millisecond at these sizes with cache locality on your side. For the sizes swaypplet works at (bar ≈ 2880×40 = 0.12 Mpx, notification ≈ 400×120 = 0.05 Mpx, lock card ≈ 500×350 = 0.18 Mpx) both are deep in the noise, well under 0.1 ms.

**The real cost is not the shader, it is the re-render policy.** hyprglass [issue #59](https://github.com/hyprnux/hyprglass/issues/59) is the honest statement of it: the effect is computed once and cached while windows are static, so "there is no effect re-rendering when window in background have internal content changes, like it's the case when videos are playing... This is initialy a design choice to have near 0 GPU usage for the glass rendering, however it may be uncanny in term of UX." Same tension in niri, whose default x-ray blur pre-blurs the wallpaper once and reuses it, versus normal blur that reads mid-frame. scenefx has the same split (`use_optimized_blur`). A live refraction that tracks a video playing behind your bar costs you a full-region recomposite every frame; a cached one is free and occasionally wrong. Note also that the `liquid_glass` path always does the `fx_renderer_read_to_buffer` copy with no caching, so it is currently on the expensive side of that choice.

Other real-world caveats worth reading before committing, all from hyprglass's tracker: [#58](https://github.com/hyprnux/hyprglass/issues/58) jagged rounded corners on translucent notifications (the layer-mask approach), [#56](https://github.com/hyprnux/hyprglass/issues/56) artifacts and mirroring, [#54](https://github.com/hyprnux/hyprglass/issues/54) effect drops on fake-fullscreen. The scenefx-enhanced commit log shows the same class of bugs being chased.

---

## 6. Recommendation

Two independent tracks, neither blocking the other.

**Lock screen, client-side, no compositor changes.** Replace the `push_blur(28.0)` in `GlassPane` with `backdrop-filter` on `.lock-card` carrying an inline `data:` SVG filter that chains `feGaussianBlur` → `feDisplacementMap` (displacement map generated per kube.io's math, or drawn as an `feImage`/`feTurbulence`-free precomputed PNG) → `feSpecularLighting`. GTK copies the wallpaper `Picture` beneath and displaces it. This is the highest-value, lowest-risk change: it is the one surface where you own the backdrop, GTK 4.22.4 is already installed, and no fork of anything is involved. Cost me one caveat: `filter`/`backdrop-filter` are `GTK_STYLE_PROPERTY_ANIMATED`, so the sigma ramp `glass.rs` currently does by hand may become a CSS transition, but interpolating between two `url()` filters is only defined when both sides are the same filter (`gtk/gtkcssfiltervalue.c:569-576`), so verify the enter animation still ramps.

**Layer-shell surfaces, compositor-side.** The work is done and it targets swayfx specifically. The move is to build `scenefx-enhanced` + `swayfx-enhanced` and evaluate, rather than write a shader. Risks to weigh: it is a single-maintainer fork of a fork, marked Experimental with active artefact-fixing commits, its flake pins upstream scenefx so packaging needs a redirect, and swayfx-enhanced carries unrelated behaviour changes (scrollable tiling, dimming, scratchpad-as-minimize) that you would be adopting alongside. The narrower alternative is cherry-picking `liquid_glass.frag` and `fx_render_pass_add_liquid_glass()` into your own scenefx pin, since the shader is self-contained and the pass function is ~90 lines.

**Working checkouts** (scratchpad, not in your repo): `/tmp/claude-1000/-home-meros-nixos/4e41c2b1-fc55-439e-bb6f-2300149ef4ef/scratchpad/` contains `scenefx/`, `scenefx-enhanced/`, `swayfx-enhanced/`, and `gtk4src/` (extracted GTK 4.22.4 source).

---

## Status

Research only, August 2026. Nothing in this document is implemented and no
decision has been taken. Recorded so the next person (or the next session)
does not have to rediscover that `GskGLShader` is dead, that GTK 4.22 quietly
gained a usable replacement, or that the compositor-side work already exists
for swayfx specifically.

The working checkouts referenced at the end lived in a scratch directory and
are gone. The URLs in the tables are the durable record; re-clone if needed.

Two things to re-verify before acting, because both are version-sensitive and
both were read from source rather than from documentation:

- That `backdrop-filter` with an inline `data:` SVG `feDisplacementMap` really
  renders under the GTK version in the flake at that time. It was verified to
  exist and to parse in 4.22.4; it was not run.
- That `scenefx-enhanced` still carries `liquid_glass.frag` in a form worth
  cherry-picking, and what it has drifted into. It was experimental and under
  active artefact-fixing when this was written.
