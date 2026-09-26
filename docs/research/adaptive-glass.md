# Glass that responds to what is behind it

**Recommendation: build none of the per-surface or polarity-flipping variants; if anything, try one shader-only experiment in which a "busyness" measure (detail left after the narrow frost) raises `extra_blur` and, within a cap, `fill_alpha`, and is zero over a flat backdrop, so every APCA test holds unchanged.**

Research, 2026-09-26. **[src]** verified in source or at the link; **[judgement]** my reasoning, unmeasured.

## 1. What photochromic already covers, and the gap it leaves

- **[src]** `liquid_glass.frag` applies photochromic per pixel to transmitted luminance: ceiling `C·(1−e^(−L/C))` in dark, lift `L + f·e^(−L/f)` in light (`scenefx-photochromic-lift.patch`). The glass already adapts to backdrop *brightness*.
- **[src]** `tokens::material::glass_body` takes one flat `Rgb`; all 10,440 APCA sets are tested over uniform backdrops.
- **[judgement]** The gap is **structure, not level**. Photochromic is a pointwise tone curve: it cannot tell a flat grey desktop from a code editor with the same mean luminance. The editor's detail shows through at whatever `frost` the mode sets, and the tests never see it. High-frequency energy that survives the frost is what made cards unreadable before (`liquid-glass.md`, "The frost is a blur": 2.6× on a code editor). What content-adaptive glass adds: more frost or fill **where the backdrop is busy**.

## 2. Prior art

| System | What adapts | Source |
|---|---|---|
| Apple Liquid Glass (iOS/macOS 26) | Tint and dynamic range "shift to always ensure buttons remain legible, while letting as much of the content through as possible". The shadow gets more opaque "when it is over text" and less opaque over a solid light background. Small elements "flip from light to dark based on the background", and their glyphs flip with them. Large elements (menus, sidebars) "don't flip… transitions like these would be distracting". **[src]** | [WWDC25 219](https://developer.apple.com/videos/play/wwdc2025/219/) |
| Apple, iOS 26.1 | User toggle Clear/Tinted; Tinted "increases opacity and adds more contrast". Global, not content-driven. **[src]** | [MacRumors](https://www.macrumors.com/how-to/ios-26-1-reduce-liquid-glass-effects/) |
| Windows Acrylic | Fixed recipe; luminosity blend evens lightness, `TintLuminosityOpacity` derived from tint, not content. **[src]** | [Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic) |
| Windows Mica | "Only samples the desktop wallpaper once"; solid fallback on battery saver or inactive windows. **[src]** | [Mica](https://learn.microsoft.com/en-us/windows/apps/design/style/mica) |
| KDE KWin | "Background contrast": static contrast/intensity/saturation per region (`org_kde_kwin_contrast`). **[src]** | [contrast.xml](https://github.com/KDE/plasma-wayland-protocols/blob/master/src/protocols/contrast.xml) |
| GNOME / `ext-background-effect-v1` | Blur only. The protocol's single capability is `blur`. **[src]** (`research/liquid-glass.md` §4) | [Phoronix](https://www.phoronix.com/news/Wayland-Background-Effect) |
| Leykin & Tuceryan, ISMAR 2004 | A classifier trained on texture features of the backdrop predicts text readability (>85 %). "Background variations only affect readability when the text contrast is low." **[src]** | [Semantic Scholar](https://www.semanticscholar.org/paper/Automatic-determination-of-text-readability-over-Leykin-Tuceryan/34c215f443291f0634f213c1962f02ee4920fcfc) |
| Gabbard, Swan & Hix, Presence 2006 | The "billboard" style (text on a semi-opaque plane) isolates text from a textured background most reliably. **[src]** | [MIT Press](https://direct.mit.edu/pvar/article/15/1/16/18626/The-Effects-of-Text-Drawing-Styles-Background) |

**[judgement]** Shipping systems adapt *level*, which photochromic already does; only Apple reacts to structure (text behind), and only via its shadow. The research says busyness hurts mainly where contrast is already marginal, which for us is bright or busy backdrops.

## 3. Options for this stack, ranked by value for cost

GPU numbers are **[judgement]** estimates. Baseline from source **[src]**: GLSL ES 1.00 (`precision` prelude, `texture2D`), 9 seed taps plus `samples` (4) spectral backdrop fetches, plus scattered, reflection and mask fetches. That comes to roughly 20–30 fetches per fragment, over three buffers the size of the output (sharp, `tex_frost`, `tex_frost_wide`).

**(a) Per-pixel busyness in the shader. Value: medium. Cost: low.** Take `b = sqrt(mean_i (L(frost(p+r·d_i)) − L(frost_wide(p)))²)` over 4 taps of `tex_frost` on a ring of radius about `frost_radius`. `frost_wide(p)` is already fetched for the haze. This is local RMS contrast of the detail that survives the frost, which is what competes with text.
- Map `b` to `extra_blur` (already a parameter of `backdrop_at`, costs no new passes) and to `fill_alpha' = fill_alpha + k·smoothstep(b0, b1, b)`, capped.
- Cost: 4 extra fetches, about +15–20 %. For a 600×400 card that is about 1 M extra texel fetches per frame, well under 0.1 ms on an iGPU against a 16.7 ms budget at 60 fps.
- No mips are needed. They would be awkward anyway: GLSL ES 1.00 fragment shaders have no `texture2DLod` without `EXT_shader_texture_lod`, and `glGenerateMipmap` on an output-sized buffer every frame costs more than the 4 taps. The two frost levels already act as a two-level pyramid.
- Risk: shimmer while the backdrop scrolls. `b` is spatially smooth; temporal smoothing would need a history buffer, not worth it.

**(b) Per-surface mean or variance, reduced on the GPU. Value: low to medium. Cost: medium.** Reuse the bounds pass's columns-then-combine reduction **[src]** (`mode 0/1`, `card_bounds`). It can write `mean(L)` and `var(L)` under the card into one texel that the glass pass samples. This needs no CPU readback and no stall.
Buys uniformity (whole card shifts together) and a lever to close §5's white-page exception (`--fg` Lc 62) without lowering the ceiling for mid-tones. Costs a reduction pass and hysteresis against pumping.

**(c) Text-aware, only under the card's own text. Value: low. Cost: medium.** The shader samples the client buffer (`mask_sample_raw`) **[src]**, so any pixel that is not the key colour is content. "Near text" needs 8+ mask taps or a blurred coverage channel in the flood; our cards are text-dense, so the win is small.

**(d) Flip text polarity per surface, Apple style. Value: low. Cost: high, and it conflicts with the token model.**
- Mode is one global input (§2.1) with tokens proven against that mode's material; a per-surface flip means two live token sets and per-namespace mode.
- Needs compositor-to-client readback (PBO, new IPC event, CSS reload): frames of latency plus hysteresis.
- Apple limits flipping to small elements; nearly all our surfaces are large.

## 4. Keeping the APCA guarantee

**[judgement]** This is safe by construction if three conditions hold:

1. **The adaptive term is zero over a flat backdrop.** Flat means `frost == frost_wide`, so `b = 0`. `glass_body` still models the shader exactly, and all 10,440 sets hold unchanged.
2. **It only moves the body toward the fill.** It raises `fill_alpha` and blur, never the reverse. APCA Lc is monotonic in background Y for fixed text, so as long as the polarity does not change, a body between "flat worst case" and "pure fill" scores at least the lower of the two.
3. **The cap is a number already shipped: `fill_alpha ≤ 0.72`** (high contrast). Add one test that each mode's text tokens pass over `fill_color` at alpha 0.72 and alpha 1, and that the body stays on the same side of the text's luminance.

## 5. Minimal experiment

1. In `liquid_glass.frag`, compute `b` from 4 `tex_frost` taps (section 3a) and feed `max(extra_blur, g·b)` into `backdrop_at` behind a new `liquid_glass_adapt` uniform. Default 0, so nothing changes.
2. Add debug mode 6 to draw `b`. Check that it is black over a flat wallpaper and bright over a terminal or code editor.
3. Measure the high-frequency energy of the body (the RMSE-crop method already in `liquid-glass.md`) over a code editor at `adapt` 0 and 1, and time the frame with the existing perf patch's instrumentation.
4. Only if the blur alone is not enough, add the capped `fill_alpha` term and the test from section 4.

About 20 lines of GLSL and one uniform. Stop if step 3 shows no visible win: photochromic already covers luminance.
