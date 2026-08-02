# Bar ideas — research and near-term specs

> Two research passes from the bar-integration project (2026-08-02): a survey of
> prior art worth stealing, then concrete specs sized for implementation.
> Grounded in swaypplet's constraints: swayfx binary frost, anim.rs motion
> system, one process owning bar + panel + OSD + notifications.

## Survey: what others do

**Ironbar** (Rust + GTK4, the closest architectural cousin) pairs every bar module with an optional anchored popup: clock → calendar, music → full controls with art and a seek bar, upower → detail readout. Its two most-copied ideas are the popup-per-module pattern and *ironvars*, a tiny IPC key-value store so external scripts can push content into custom modules at runtime without the bar knowing about them ([Ironbar repo](https://github.com/JakeStanger/ironbar), [custom module docs](https://ironb.ar/modules/custom/custom/)).

**SketchyBar** (macOS) is the reference for event-driven bars: items subscribe to system events (app focus change, volume, display reconnect) and run callbacks that can add, remove, restyle, or animate items at runtime. It has a first-class animation system (position, size, color tweens) and on-demand popup menus attached to any item, and its community configs lean hard into per-app context — the bar reshapes around the focused application ([SketchyBar](https://felixkratz.github.io/SketchyBar/), [repo](https://github.com/FelixKratz/SketchyBar)).

**eww / AGS-Astal setups** live on progressive disclosure: the canonical pattern is a collapsed icon that expands on hover via a revealer (slideleft/slideright/crossfade transitions) into controls — volume sliders, music pills growing into artist–title with buttons, focused workspace widening while others shrink to dots ([eww widget docs](https://elkowar.github.io/eww/widgets.html), [example config](https://github.com/owenrumney/eww-bar)). **HyprPanel** shows where full-shell bars converge: clock opens a calendar+weather menu, media module opens a player card, notifications get a bell with counts and DND toggle, plus a built-in OSD — everything is a menu hanging off a bar segment ([HyprPanel](https://github.com/Jas-SinghFSU/HyprPanel), [panel config](https://hyprpanel.com/configuration/panel.html), [OSD](https://hyprpanel.com/configuration/osd.html)).

**macOS Dynamic Island / Live Activities** contributes the discipline, and it matches swaypplet's existing motion philosophy: a persistent surface is a *status layer*, worth occupying only for things the user repeatedly checks; compact and expanded presentations must keep the relative placement of elements so the morph reads as one object; updates animate only on meaningful state transitions, never on ticks ([WWDC23 "Design dynamic Live Activities"](https://developer.apple.com/videos/play/wwdc2023/10194/), [Apple's Live Activities guidance](https://developer.apple.com/news/?id=bkm73839)). When two activities compete, the island splits into a primary lozenge plus a detached mini-pill rather than interleaving them. macOS ports of the idea (Atoll, Alcove) confirm the pattern survives on a desktop bar ([Atoll](https://github.com/Ebullioscopic/Atoll)).

**GNOME Shell** is the counter-voice on attention: notifications got app-attributed headers (anti-impersonation), app-based grouping with batch dismissal, and a push to make urgency and DND centrally enforced — organization over animation, and only critical events (battery) are allowed to break through DND ([Notifications in 46 and beyond](https://blogs.gnome.org/shell-dev/2024/04/23/notifications-46-and-beyond/)).

**polybar/dwm power users** keep proving the demand for a tiny interactive timer segment: click to start a pomodoro/tea timer, scroll to adjust, with the countdown ambient in the bar ([polybar-timer](https://github.com/jbirnick/polybar-timer), [polypomo](https://github.com/unode/polypomo)).

## Ideas worth stealing

**1. Popups anchored to bar segments (Ironbar/HyprPanel pattern).** Clock → calendar card, battery → power detail, media → art + seek. Each popup is its own bottom-anchored layer surface styled `.glass-card` (data/style.css:63), entering via `anim::Reveal` + `SlideBin` exactly like the panel — the binary-frost rule is already solved there (pane tint lands in `GLASS_MS`, content fades over `ENTER_MS`, anim.rs:40–53). The building blocks all exist: `layer_shell::create_layer_window_on` with per-output margins (bar/mod.rs:139), and `MediaSection` already resolves album art, position, and length (widgets/media.rs:60–84). Feasibility: high; the one design constraint is no drop shadows on the popup (alpha>0 pixels frost, style.css:71–77), so depth must come from the hairline border, which the panel already demonstrates.

**2. Progressive-disclosure media pill.** Collapsed: icon + ellipsized title (current bar/media.rs). On hover, a `GtkRevealer` (200 ms structural scale, per anim.rs module header) slides in prev/play/next buttons and a thin progress hairline; on click, the full popup from idea 1. Crucially the morph happens *inside* the always-mapped bar card, where alpha stays 1 everywhere, so swayfx's binary frost never enters the picture — width changes are pure GTK layout. The `CenterBox` (bar/mod.rs:154) guarantees the pill grows symmetrically without shoving the clock. Follow Apple's coherence rule: icon and title keep their relative placement between collapsed and expanded states.

**3. Live-Activity discipline for the task pill.** The task pill (bar/task.rs) is a homegrown Live Activity: persistent, glanceable, state-bearing. Steal the animation policy: animate only meaningful transitions — a session flipping to `Waiting` (needs the user) gets a single accent pulse in the "deliberate attention loops" bucket the CSS header already carves out (style.css:11–13), while `Working` stays perfectly still. The `PillView` equality cache (task.rs:122–128) already identifies exactly the frames where a real transition happened, so triggering a one-shot CSS animation on change is nearly free. GNOME's urgency discipline applies: only `Waiting` and battery-critical earn motion; everything else just changes.

**4. Per-app context segment (SketchyBar's best trick).** A focused-window module: app icon + title, and for known apps a contextual affordance (browser → profile/PWA name, terminal on a task workspace → the task accent it already gets via `bar-taskN` root classes, task.rs:282–290). `SwayService` already streams focus/title events per keystroke (media.rs:36–38 leans on this), and `icons.rs` handles lookup. Feasibility: high for title display, medium for per-app actions; start read-only and let click focus/cycle windows. Keep it in the left cluster so the center pill stays reserved for transient activities.

**5. The "two activities" split.** When media and a Claude task both demand attention, adopt the island's primary + detached-mini rule instead of concatenating text: the center pill shows the *active* concern (a `Waiting` task outranks a playing track), and the demoted one collapses to a bare icon-dot segment. The priority function is trivial since both states are already comparable snapshots (`PillView`, `MediaState`). This is a policy layer, no new surface tech — just visibility and `GtkRevealer` swaps inside the bar card, animated at the 200 ms structural scale.

**6. Ambient progress on borders.** Claude sessions already publish `progress-<PID>` ("3/5", task.rs:223) and media has position/length; render them as a hairline progress fill along a pill's top border rather than text. The border-top accent channel already exists — `bar-taskN` stamps recolor `.bar-seg` top borders via descendant selectors (style.css:292–295) — so a progress variant is one custom-drawn 2 px overlay widget (a sibling of `SlideBin`'s snapshot approach, anim.rs:267–277) inside the pill. Calm by construction: no text changes, no reflow, sub-pixel motion at update time only.

**7. Timer segment in the instrument track.** polybar-timer semantics fit the existing right track (`.bar-track`, bar/mod.rs:171–188): click starts a preset, scroll adjusts, countdown text ambient, one accent pulse on expiry, then a notification through the in-process store (`NotificationStore`). Clock.rs's boundary-aimed one-shot timers (clock.rs:3–6) are the right ticking model — re-aim at the next second only while a timer runs. Cheap to build, and it's the single most-requested "bar as instrument" feature in the polybar/dwm world.

**8. Workspace morph: focused-grows, rest-shrink.** The eww/AGS pattern where the focused workspace pill widens (icon + label) while unfocused ones collapse to dots. Workspaces.rs already renders per-workspace buttons with task-colored dots and superscripts; wrapping each label in a `GtkRevealer` with the shared 200 ms duration gives the morph, and since the whole cluster lives in the left `CenterBox` slot, growth never moves the centered media pill (bar/mod.rs:152–157). One caution from the same file's design history: sway fires events per keystroke, so gate the reveal on actual focus *changes* (cache-compare, as task.rs does) or the bar will shimmer.

**9. A state-dir contract as the "ironvars" equivalent.** Ironbar's script-injection IPC maps onto a pattern swaypplet already proved: the `~/.local/state/claude-tasks` file contract watched by `GFileMonitor` (task.rs:139–152). Generalize it: any script drops `~/.local/state/swaypplet/segment-<name>` (text + optional state class) and a generic segment appears in the track, no bar changes needed. This keeps swaypplet config-in-code while giving the nixos repo's shell scripts a sanctioned way to surface state, and inotify-driven updates cost nothing at idle.

Sources: [Ironbar](https://github.com/JakeStanger/ironbar) · [Ironbar custom modules](https://ironb.ar/modules/custom/custom/) · [SketchyBar](https://felixkratz.github.io/SketchyBar/) · [SketchyBar repo](https://github.com/FelixKratz/SketchyBar) · [eww widgets](https://elkowar.github.io/eww/widgets.html) · [eww-bar example](https://github.com/owenrumney/eww-bar) · [HyprPanel](https://github.com/Jas-SinghFSU/HyprPanel) · [HyprPanel panel config](https://hyprpanel.com/configuration/panel.html) · [HyprPanel OSD](https://hyprpanel.com/configuration/osd.html) · [WWDC23: Design dynamic Live Activities](https://developer.apple.com/videos/play/wwdc2023/10194/) · [Apple: Explore Live Activities](https://developer.apple.com/news/?id=bkm73839) · [Atoll (Dynamic Island for macOS)](https://github.com/Ebullioscopic/Atoll) · [GNOME Shell: Notifications in 46 and beyond](https://blogs.gnome.org/shell-dev/2024/04/23/notifications-46-and-beyond/) · [polybar-timer](https://github.com/jbirnick/polybar-timer) · [polypomo](https://github.com/unode/polypomo)
## Near-term specs

All paths relative to `/home/meros/git/personal/swaypplet`. Shared constraints that shape every spec: layer surfaces never resize after map (panel.rs:103-107), so all motion is render-node motion (opacity, `SlideBin` translation, clipped drawing); pane tint always rides `glass_channel` within `GLASS_MS` (90 ms) while content fades over `ENTER_MS`/`EXIT_MS` (300/200 ms, `ease_out_cubic`); every entry point checks `anim::animations_enabled()`.

### 1. Bar-to-panel morph

**Look.** Pressing the start button (bar/start.rs), the panel card appears to grow out of it: a small rounded seed rectangle at the button's screen position (bottom-left, ~36×30 px inside the 38 px bar) expands to the full 780×700 card while the frost tint lands in the first 90 ms. Exit reverses: content gone first, card shrinks back into the button.

**Mechanism.** The panel window already covers the work area (its backdrop is hexpand/vexpand, panel.rs:82-88), so the whole morph happens inside one fixed-size surface. New primitive `MorphBin`, a sibling of `SlideBin` (anim.rs:238-336): `snapshot()` pushes a rounded-rect clip plus translate+scale interpolated between a `seed: graphene::Rect` and the child's resting bounds. Seed coordinates come from the start button via `compute_bounds` against the bar window plus the bar's 4 px margins (bar/mod.rs:50), passed through the existing `toggle_panel: Rc<dyn Fn()>` hook (bar/mod.rs:67-68), widened to carry an origin rect.

**States.** closed → morph-in (ENTER_MS: clip/scale eased, pane tint in first GLASS_MS, content opacity held 0 for the first ~40 % then fading) → open → morph-out (EXIT_MS mirrored; pane drops to exactly 0.0 in the last GLASS_MS per the swayfx stencil rule). Retrigger mid-flight retargets from current progress, same pattern as `Reveal::animate`. Reduced motion: jump, exactly like `Reveal`. The start button holds a `.morph-origin` class (dimmed) while open.

**Size.** ~130 LoC `MorphBin` in anim.rs, ~50 LoC panel/bar wiring, ~15 lines CSS. Replaces the current `SlideBin` settle path in `Panel::new` (panel.rs:317-328) behind a fallback: no origin rect → today's fade+settle.

### 2. Per-widget popovers reusing panel sections

**Look.** Clicking a right-track segment opens a compact glass card (width ~340, matching the panel's right column at panel.rs:137) floating 8 px above the bar, horizontally centered over the segment and clamped to screen edges. Battery → `PowerSection` (`expand_for_page`, widgets/power.rs:588), bar media pill → `MediaSection`, bell (spec 3) → `NotificationsSection`, bar toggles (spec 4) right-click → `NetworkSection`/`BluetoothSection`.

**Mechanism.** One `PopoverHost` per output: a lazily created Overlay-layer window anchored bottom, with `Edge::Left` margin computed from the segment's `compute_bounds` (same coordinate trick as spec 1). Singleton: opening one closes any other. Each popover constructs its **own** section instance; sections must never be shared with the panel because widgets have one parent and the panel already hoists scales via unparent (panel.rs:444-452). `refresh()` runs on every open, mirroring `Sections::refresh` (panel.rs:47-61).

**States.** closed / open / repositioning (click a different segment while open: exit EXIT_MS, then enter at the new anchor). Backdrop click, Esc, and a second click on the segment all close. Segment gets `.popover-open` (same visual weight as `.toggle-btn.active`).

**Motion.** `Reveal` + `SlideBin` with `SLIDE_PX` (24 px) up-settle, identical to the start menu, so popovers read as detached panel fragments.

**Size.** ~200 LoC `src/popover.rs` host, ~15 LoC per wired segment. Battery first (its section is pure status), media second.

### 3. Notification bell + DND

**Look.** New right-track segment between battery and task pill: 󰂚 plus a count badge when unread > 0; DND swaps to 󰂛 with a dim `.dnd` class and hides the badge; any Critical notification present forces `.critical` (reuses the battery pulse styling, bar/battery.rs:58).

**State source.** `NotificationStore` (notifications/store.rs): `connect_notify` + `connect_change` drive refresh; ids are monotonic (store.rs:138), so unread = count of `store.all()` ids above a bell-local `last_seen_id` watermark, set when the popover opens. No store changes needed. One gap to fix: `set_dnd` (store.rs:107-109) fires no change callbacks, so the panel DND tile and the bell would drift; add `collect_change` to `set_dnd` so both refresh from one source.

**Interaction.** Left click → popover (spec 2 chassis) hosting a fresh `NotificationsSection` with `set_list_max_height(400)` and clear-all; opening advances the watermark. Right click toggles DND directly.

**Motion.** New notification (DND off): badge count pops via 150 ms CSS micro-transition and the glyph gets a one-shot nudge, `SlideBin` dy 0→−3→0 using `slide_to` twice (or a 3-keyframe CSS animation; CSS auto-honors `gtk-enable-animations`). Waiting-state breathing is spec 6's pattern; here the pulse is reserved for Critical.

**Size.** ~150 LoC `src/bar/bell.rs`, ~10 LoC store change, CSS ~20 lines.

### 4. Declarative bar toggles from `TileSpec`

**Look.** Icon-only 24 px segments in the right track (glyph from `spec.icon`, tooltip from `tooltip_on/off`), default set: Night Light + Caffeine (Wi-Fi/BT stay in panel and popovers where their device lists live). Active = accent-filled like `.toggle-btn.active`; `Unavailable` hides the segment entirely (bar width is precious; the panel keeps the disabled affordance).

**Mechanism.** `TileSpec` is already `Copy`-cheap and side-effect-free (tiles.rs:28-38), so the bar variant is a second factory: extract the optimistic-toggle/revert/`loading` wiring from `build_tile` (tiles.rs:110-129) into `wire_toggle(btn, action, tooltips)` and call it from both. State init via existing `init_tile_state`; re-read on a 60 s timer plus after each action completes (the completion callback already exists).

**States.** Active / Inactive / loading (existing `.loading` class, rendered as 0.6 opacity pulse) / revert (failed action snaps back after 2 s, tiles.rs:122-126, unchanged).

**Motion.** Micro only: 150 ms CSS color transitions. No structural motion; segments are permanent bar residents.

**Size.** ~80 LoC `src/bar/toggles.rs` + ~30 LoC shared-wiring refactor in tiles.rs.

### 5. OSD-on-the-bar

**Look.** Volume/brightness keys animate a transient segment sliding open at the left end of the right track: icon + 64 px progress bar + "45%". The center-screen card (osd.rs) remains for fullscreen windows and for lock indicators (caps/num have no bar segment).

**Routing.** `Osd::trigger` (osd.rs:336-345) keeps executing the command on a worker; `show_display` gains a route: if the focused output has a bar and the focused window is not fullscreen → bar segment on that output; else center card. Needs one `SwayService` extension: a `fullscreen` flag on the focused-window snapshot (from `get_tree` `fullscreen_mode`), plus focused-output lookup which the workspace cache already implies (sway_ipc.rs).

**States.** collapsed / expanding / shown (retrigger resets the 1500 ms timer, `OSD_TIMEOUT_MS`, same `timeout_id` pattern as osd.rs:386-401) / collapsing. Mute renders fraction 0 + "Muted" text, matching `read_volume_display` (osd.rs:144-175).

**Motion.** `GtkRevealer` `SlideLeft` at the structural 200 ms (the one sanctioned revealer duration, anim.rs module header); the progress fraction animates from its previous value over 150 ms with `ease_out_cubic` via a tick callback so repeated key presses read as one continuous sweep rather than steps. No glass work needed: the segment lives on the existing bar pane.

**Size.** ~140 LoC `src/bar/osd_seg.rs`, ~30 LoC routing in osd.rs, ~25 LoC sway_ipc fullscreen field.

### 6. Task pill innovations

Three layered upgrades to bar/task.rs, all fed by the existing state contract (`pid-<PID>`, `status-<PID>`, `progress-<PID>`, `manual-t<N>` under `~/.local/state/claude-tasks`, task.rs:1-17).

**a. Progress ring on the activity dot.** Parse a leading `N/M` from `progress-<PID>` ("1/5 ETA ~15m" → 0.2). Replace the ● Label (task.rs:300-303) with a 12 px custom widget: cairo arc in `snapshot()`, dot in the center keeping the working/waiting/stopped color classes. Fraction changes sweep over `MOVE_MS` with `ease_out_cubic`; Working with no parseable fraction shows a slow indeterminate arc (tick-driven rotation, gated on `animations_enabled`). Non-fraction progress text still renders after the "·" as today.

**b. Attention pulse on waiting.** On a working→waiting transition (detectable in the refresh closure by diffing against the cached `PillView`, task.rs:114-129), the pill does a one-shot `SlideBin` nudge (dy 3 px settle, 200 ms) and the dot gets `.attention`: a 2 s CSS breathing loop that runs until the state changes or the pill is clicked. Waiting means Claude needs input; the bar should say so without a center-screen interruption.

**c. Per-workspace task ribbons.** Task workspace buttons (nums 1-16, workspaces.rs:104-113) get a 2 px bottom ribbon showing that task's aggregate session state across all outputs: working = solid accent, waiting = breathing, none/stopped = off. Requires lifting the session scan (`read_view`'s pid/comm//proc walk, task.rs:189-232) out of the per-output pill into a shared `TaskStateService` beside `SwayService`: one `GFileMonitor`, one scan per FS event, observers on both the pills and the workspace modules. Also removes today's duplicate scans per output.

**Size.** service ~120 LoC (`src/task_state.rs`), ring widget ~80, ribbons ~40, pulse CSS ~15. Ship in that order; a and b need no service.

### 7. Clock calendar popover (own)

Click on the clock currently toggles time/date in place (bar/clock.rs:44-50). Instead: click opens a spec-2 popover with a month grid (7×6 `gtk4::Grid`, today accent-ringed, header with month/year and ‹ › buttons), keeping the in-place date toggle on right-click for muscle memory. Pure `glib::DateTime` math, no new dependencies. States: current month / browsing (a "today" tap snaps back). Motion: the chassis Reveal; month changes crossfade the grid over 150 ms. ~150 LoC `src/widgets/calendar.rs`.

### 8. Media pill progress underline + hover controls (own)

The center media pill (bar/media.rs) gets a 2 px underline tracking playback position (playerctl position/length, polled 1 s while playing, fraction eased 150 ms per update like spec 5's bar). Hover overlays ⏮ ⏯ ⏭ on the pill using the `wrap_with_chevron` Overlay pattern (tiles.rs:145-158): buttons fade in 150 ms CSS, take no layout width, and the title dims under them. Click without hover-dwell still does today's action. States: playing (underline advances) / paused (underline frozen, `.paused` dim) / none (pill hidden as today). ~90 LoC in bar/media.rs.

### 9. Sliding focus caret for workspaces (own)

Workspace focus changes currently rebuild buttons and the `.focused` class snaps (workspaces.rs:69-93). Add a floating 3 px accent caret under the button row that **slides** to the newly focused button over `MOVE_MS`. Mechanism: generalize `SlideBin` to a 2-axis offset (`dx` joins `dy`, anim.rs:243-277; `slide_to` gains an axis or takes a Point), then park the caret in an Overlay spanning the workspace box, retargeting to `compute_bounds` of the focused button on each sync. Task workspaces color the caret with their `TASK_DOT_COLORS` entry (workspaces.rs:17), so focus motion doubles as a task-color trail. Rebuilds jump the caret (no animation across a structural rebuild). Reduced motion: jump always. ~60 LoC anim.rs generalization (also unlocks horizontal motion for spec 1's seed travel), ~50 LoC workspaces.rs.

**Suggested order.** 4 → 3 → 2 → 6a/6b → 5 → 9 → 6c → 8 → 7 → 1. The toggle refactor and bell are small and independent; the popover chassis unblocks three others; the morph is the flagship but wants the anim.rs generalization from 9 first.