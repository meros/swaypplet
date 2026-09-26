# Motion, rendering and performance: prior art

**45 entries. The field has converged on three things swaypplet only half has: springs that keep velocity when interrupted, animation evaluated off the app's main thread, and frame gates that measure jank relative to the refresh rate and attribute it to a layer of the stack.** Each entry marks its sources [verified] or [memory]; check [memory] figures before quoting them.

## Landscape

- **Timing models.** Duration-plus-curve tokens ([Material](material-motion-easing-tokens.md), [Fluent](fluent-motion-timing.md)) are giving way to springs ([SwiftUI](swiftui-spring-animations.md), [M3 Expressive](m3-expressive-spring-motion.md), [niri](niri-spring-animations.md), [Hyprland](hyprland-animations.md), [libadwaita](gtk-css-animation-costs.md)), with the curves kept for opacity and colour.
- **Where animation runs.** Out of the app: [Core Animation](core-animation-render-server.md), [Chrome's compositor](chrome-compositor-thread.md), [Qt Animators](qt-quick-scene-graph.md), [RenderThread](android-choreographer-renderthread.md). GTK4 has no such path ([GSK](gtk4-gsk-unified-renderers.md)).
- **Frame delivery.** [Mutter's frame clock](mutter-frame-clock.md), [dynamic triple buffering](mutter-dynamic-triple-buffering.md), [ProMotion](ios-promotion-adaptive-refresh.md), [VRR](variable-refresh-desktop.md), [Swappy](android-frame-pacing-swappy.md).
- **Blur cost.** [Dual Kawase](dual-kawase-blur.md) in [KWin](kwin-blur-dual-kawase.md), [Hyprland's caching](hyprland-blur-optimizations.md), [Mica](windows-acrylic-mica.md), [Apple materials](apple-vibrancy-materials.md).
- **Measurement.** [JankStats](android-jankstats.md), [FrameTimeline](android-frametimeline.md), [Macrobenchmark](android-macrobenchmark.md), [Chromium perf](chromium-perf-dashboard-pinpoint.md), [Typometer](typometer.md), [Is It Snappy](is-it-snappy.md).
- **Humans.** [100 ms](response-time-limits.md), [Doherty](doherty-threshold.md), [touch latency](low-latency-touch-research.md), [reduced motion](prefers-reduced-motion.md), [WCAG 2.3.3](wcag-animation-from-interactions.md), [Chang and Ungar](chang-ungar-cartoon-animation.md).

## 10 best ideas

1. **Retarget with velocity kept** ([SwiftUI](swiftui-spring-animations.md), [Fluid Interfaces](designing-fluid-interfaces.md)).
2. **Spatial springs may bounce; effects springs never do** (damping 1.0 for opacity and colour) ([M3 Expressive](m3-expressive-spring-motion.md)).
3. **Animation as committed data, evaluated by the compositor** ([Core Animation](core-animation-render-server.md)).
4. **Jank measured against the refresh interval**, 2x by default ([JankStats](android-jankstats.md)).
5. **Expected vs actual present, blamed on app, compositor or display** ([FrameTimeline](android-frametimeline.md)).
6. **Each frame tagged with what was animating** ([JankStats](android-jankstats.md)).
7. **Expensive material with a defined cheap twin**, switched on battery or by a user setting ([Acrylic/Mica](windows-acrylic-mica.md), [battery-aware](battery-aware-animation.md)).
8. **Blur at quarter resolution** (dual Kawase), judged by bandwidth ([dual Kawase](dual-kawase-blur.md)).
9. **Shaders compiled ahead of first use** ([Impeller](flutter-impeller.md)).
10. **Reduced motion replaces movement with a dissolve rather than removing it** ([WebKit](prefers-reduced-motion.md)).

## 8 instructive failures

1. Mutter's triple buffering took 4+ years to land, and an iGPU janked because it was *under*-loaded ([Mutter](mutter-dynamic-triple-buffering.md)).
2. GTK's new renderers were "not faster (yet)", with driver regressions after they became the default ([GSK](gtk4-gsk-unified-renderers.md)).
3. Acrylic made window drags lag, so Microsoft moved system surfaces to Mica ([Acrylic/Mica](windows-acrylic-mica.md)).
4. Vista's "Capable" machines could not run Aero, which led to a class action ([DWM](windows-dwm.md)).
5. Compiz made motion into spectacle ([Compiz](compiz-effects.md)).
6. Skia compiled shaders on first draw and janked ([Impeller](flutter-impeller.md)).
7. Low Power Mode at 60 Hz feels worse than a native 60 Hz phone ([battery-aware](battery-aware-animation.md)).
8. Animation scale 0 broke code that waited on animation-end callbacks ([Android scale](android-animator-duration-scale.md)).

## Ideas for swaypplet, ranked

| # | idea | size | source |
|---|---|---|---|
| 1 | Refresh-relative, attributed frame gate (see checks below) | S–M | [JankStats](android-jankstats.md), [FrameTimeline](android-frametimeline.md) |
| 2 | Run frame-bench with the shipped glass settings, not `blur_passes 1, radius 5` | S | [KWin blur](kwin-blur-dual-kawase.md) |
| 3 | Warm the glass shader and GSK pipelines at startup; add a cold first-open case | S | [Impeller](flutter-impeller.md) |
| 4 | Velocity-preserving retarget in `Reveal` (position already continues; velocity jumps) | M | [SwiftUI](swiftui-spring-animations.md), [niri](niri-spring-animations.md) |
| 5 | Solid twin per glass namespace, used for battery, `power-saver` and a Reduce Transparency setting | M | [Acrylic/Mica](windows-acrylic-mica.md), [Apple](apple-vibrancy-materials.md) |
| 6 | Under reduced motion, a 150 ms dissolve instead of a one-frame collapse for surfaces that arrive | S | [reduced motion](prefers-reduced-motion.md) |
| 7 | `slowdown` factor on `anim::duration` for debugging and filmstrips | S | [niri](niri-spring-animations.md), [Android scale](android-animator-duration-scale.md) |
| 8 | Census rule: no transitions on box-shadow, filter or layout properties | S | [GTK CSS](gtk-css-animation-costs.md), [Chrome](chrome-compositor-thread.md) |
| 9 | Commit whole animations to swayfx instead of per-frame alpha, so main-thread stalls cannot stall motion | L | [Core Animation](core-animation-render-server.md), [Qt](qt-quick-scene-graph.md) |
| 10 | Mica-style frozen frost for static namespaces (bar) | M | [Hyprland blur](hyprland-blur-optimizations.md), [Mica](windows-acrylic-mica.md) |
| 11 | Critically damped springs only where a gesture or retarget feeds velocity; curves stay for open and close | M | [niri](niri-spring-animations.md), [M3 Expressive](m3-expressive-spring-motion.md) |
| 12 | Ambient loops stop under reduced motion and after a bound | S | [WCAG](wcag-animation-from-interactions.md) |

## Checks the frame gate should add

- **Refresh-relative lateness.** Count a frame late when its interval exceeds 1.5x the output's refresh interval, not a fixed 25 ms. Run at 60 Hz and 120 Hz headless modes ([ProMotion](ios-promotion-adaptive-refresh.md)).
- **P99 of work and overrun**, alongside p50/p95/max ([Macrobenchmark](android-macrobenchmark.md)).
- **Per-animation attribution.** Tag frames in `frame_stats.rs` (`reveal:panel`, `slot:notif`) and report late frames per tag ([JankStats](android-jankstats.md)).
- **Compositor-side render time** for the same frames, with glass on, so a swayfx deadline miss is not invisible ([FrameTimeline](android-frametimeline.md), [Mutter](mutter-frame-clock.md)).
- **Interval evenness.** Count intervals that are not about one refresh, since an uneven 50 fps looks worse than an even 30 ([Swappy](android-frame-pacing-swappy.md)).
- **Duration fidelity.** Each transition's measured wall time equals its token ±1 frame, which catches drift and frame-counted stepping.
- **Continuity under interruption.** Open, then close at 50 %, then reopen. No per-frame jump over a few pixels or 0.1 alpha ([Chang and Ungar](chang-ungar-cartoon-animation.md)).
- **Cold first frame.** Measure the first open after a fresh compositor start separately, against about 100 ms ([Impeller](flutter-impeller.md), [100 ms](response-time-limits.md)).
- **Idle is zero.** With nothing moving and ambient loops off, swaypplet commits no frames and sway renders none over 5 s ([Hyprland blur](hyprland-blur-optimizations.md)).
- **Reduced motion.** Every transition finishes within one frame, or within 150 ms if it is a dissolve.
- **Keypress to frame** for the launcher, mean and max ([Typometer](typometer.md)).
- **Record the environment** (`GSK_RENDERER`, glass config, adaptive sync) and keep a per-commit history, so drift under the limits shows ([Chromium perf](chromium-perf-dashboard-pinpoint.md)).
