# Bar vision

DARK COCKPIT took the panel (30.5/40 aggregate; owner 8, deepwork 8, a11y 8, maintainer 6.5), but no panelist would ship it whole: its 16-slot glyph map drew a veto from all four reviewers, and its popover ring buffers are gold plating. Every panelist's closing hybrid converged on the same assembly, so that assembly is the vision: TOWER's board-absorbs-pill chassis (maintainer 8: "least plumbing per unit of value, no center mux"; owner: "the correct consolidation") carrying DARK COCKPIT's state semantics, above all the OFF-flag tier separating "data invalid" from "act now" (stolen verbatim by all four panels, and the fix for the task.rs:46-51 unknown-to-Waiting cry-wolf that poisons every trust model); NOMINAL's cadence and stand-down discipline (deepwork 9: "the only concept whose resting state is genuinely inert"); and two pieces of CUE, the OSD interjection in the center slot (owner: "the best single idea in all four submissions") and per-session acknowledgment via the pid-to-workspace map (maintainer: "nearly free"). Three consensus corrections from the panel override the concepts as written: no continuous breath animation anywhere (a CSS keyframe loop is per-frame wakeups for hours, the opposite of stillness), no information reachable only by hover-dwell (the owner does not hover; a11y calls it disqualifying), and task identity carried by position and numeral first, hue as reinforcement only (four Gruvbox accents converge at bay size under deuteranopia and under plain squinting).

## Design principles

Brief principles P1-P8 stand. The panel amends three and adds two.

1. **P1 Nominal is achromatic and still.** Unchanged. Color or motion anywhere on the bar is itself the message.
2. **P2 amended: motion is onset-only.** One 150-300 ms transient at the moment of a qualifying transition (working-to-waiting, battery critical, urgent workspace), then a *static* bright rest state. The 2 s luminance breath is deleted from the vocabulary: GTK drives CSS keyframes on the frame clock, so a looping "calm" animation burns frames for hours (maintainer finding 2). Escalation of an unacknowledged state is a static luminance tier step, never a loop. If a static tier ever proves too quiet, the sanctioned fallback is a boundary-aimed class toggle at ≥1 s cadence via glib timeout, never a frame-clock loop.
3. **P3 amended: position and shape first, hue as reinforcement.** Hue is task identity but never carries meaning alone: each board bay draws its task numeral, each state has a distinct shape (hollow socket, filled block, near-off dot, OFF-flag), and the whole bar must survive a grayscale filter. Red remains "act now", full stop.
4. **P4 Fixed spatial grammar.** Unchanged: slots never move; relevance controls prominence inside them. One center decision slot, priority-muxed, empty at nominal.
5. **P5 Two-layer contract.** Unchanged: ambient = at most two pre-attentive marks per segment; prose on demand. The one exception, per owner ruling: the decision slot shows the full 40-char settask description when a task is waiting (that is what the cap is for), plus its waiting age.
6. **P6 Cross-task awareness on every output.** Unchanged, and consolidated: one board is both the cross-task instrument and the per-output pill. Per-output accent ripple (`bar-task1..4` root classes) stays.
7. **P7 Designed cadence.** Unchanged, with the panel's implementation rule: frame-clock ticks only for one-shot transients and eased sweeps; all recurring timers boundary-aimed (clock.rs pattern) and alive only while their condition holds (the 1/min waiting-age tick exists only while a session waits).
8. **P8 amended: the bar displays, the keyboard acts, and nothing is hover-only.** Every popover opens by click and has the same content reachable without a pointer timer; the ambient layer always carries the triage minimum (a11y hard rule; the owner will never dwell).
9. **P9 new: data invalid is its own state.** Unknown or missing status renders as the amber hollow OFF-flag, never as Waiting, never as Working. An alert channel that cannot say "I don't know" cannot be trusted when it says "act".
10. **P10 new: every signal ships with its stand-down.** Written as a table (below), enforced in review: no loud state without a defined clearing condition, and focus-as-acknowledgment (input the owner already produces hundreds of times a day) is the default clearing mechanism.

## The bar, redesigned

38 px, bottom, per output, one always-opaque frosted card. Alpha stays 1 across every transition, so swayfx's binary frost never enters; all in-bar show/hide uses GtkRevealer at the 200 ms structural scale (not `anim::Reveal`, which is window-coupled and unmaps on `finish_hide`, anim.rs:202).

**Nominal** (browser workspace, all four tasks working, music playing):

```
╭───────────────────────────────────────────────────────────────────────────────────────╮
│ ⏻   ●1¹ 1² 1³ 1⁴  ●2¹ 2²  ●3¹   󰖟b 󰍡m                 ♪  󰁾 ▕1▁▁▕2▆▆▕3▁▁▕4 ·▏  14:32 │
╰──┬──┴───────────┬─────────┴──┬──┴────────┬───────────┬───┬──┴────────┬────────┴──┬───╯
 start   workspace map      generics   center empty  media battery   the board    clock
         (labels kept,                 = "nobody     mark  (dim      T1..T4 bays  (dim)
          2px task ribbons)            needs you"    (dim) glyph)    all quiet
```

**Alert** (task 3 waiting 12 min unacknowledged, caffeine on, battery 13 %):

```
╭───────────────────────────────────────────────────────────────────────────────────────╮
│ ⏻   ●1¹ 1² …map…   ● t3 · fix flaky auth retry · 12m   ♪ 󰅶 󰂃13% ▕1▁▁▕2▆▆▕3██12m▕4 ·▏ 14:32 │
╰────────────────────┴──────────────────────────────────┬──┬───┬───┴───────────────────╯
                     decision slot: task-hue dot +      │  │   red battery would own the
                     full 40-char settask desc + age  media caffeine  slot instead (priority 1)
                                                           hazard glyph
```

### Left cluster

- **Start button**: unchanged, dimmed to the nominal gray family.
- **Workspace map**: the existing `1¹`..`4⁴` labels and task dots stay (unanimous veto on replacing them; they are the keybindings in print). Additions:
  - **Task ribbons**: 2 px bottom ribbon per task group from TaskStateService. Off = no session; dim solid = working; task hue = waiting. Ribbon state changes only on task transitions (a few per hour), so adding it to the rebuild cache key is safe. 
  - **Occupancy ticks are deferred** until `rebuild()` (workspaces.rs:68-90) goes incremental: window events in the cache key would defeat the anti-shimmer gate the cache exists for (maintainer finding 3). Backlog, not vision.
  - Sliding focus caret (BAR_IDEAS 9) remains backlog, compatible.
- **Generic workspaces**: unchanged glyphs, dimmed.

### Center: the decision slot

Empty at nominal, and empty means "nobody needs you". Priority mux, one occupant, GtkRevealer enter/exit, zero neighbor reflow (CenterBox):

1. **Battery critical** (red): glyph + % + time-to-empty (already parsed in widgets/power.rs). Stand-down: charger connect.
2. **Oldest unacknowledged waiting session**: task-hue dot + full 40-char description + age chip. Multiple waiting: oldest holds the slot, `+1` suffix; the board already shows the rest. Stand-down: that session's workspace focused on any output.
3. **OSD interjection** (transient, self-decaying 1.5 s): volume/brightness keys render icon + eased hairline + value here instead of a center-screen card (BAR_IDEAS spec 5 mechanics, center-anchored). Interjects over occupants 1-2 and yields back. The only transient tenant; notifications and media never occupy the slot.
4. Empty.

### Right cluster, in order

- **Media mark**: single dim achromatic ♪ while a player exists, `.paused` dims further, hidden when idle. No title text, no ambient playback progress (deepwork veto: motionless means motionless). Detail (art, title, seek) in the click popover, BAR_IDEAS specs 2/8 as building blocks. Updates on player state change only.
- **Tray**: StatusNotifier `Status` filter (DARK COCKPIT's cut): `NeedsAttention` items show, `Active`/`Passive` live in the panel. Zero width at rest. No dwell-dot (deepwork veto: a hover invitation is a fidget invitation).
- **Hazard lane**: zero width when healthy. Appear-only glyphs via 200 ms Revealer: 󰅶 caffeine/idle-inhibit armed (in-process state, free), sway mode ≠ default (mode name in tooltip; `mode` event on the existing persistent IPC), failed user units count (zbus signal subscription, ships later, severable). Amber, static; none of these are red by default. Optional fifth: budget-pacer actively throttling (GFileMonitor on the 133-byte JSON, `.stale` class when mtime > 5 min), pending owner interest.
- **Battery**: dim gray glyph at rest, no %, no hue (owner veto on the hairline: "a dim glyph costs nothing and reads without a pointer"). Charging: bolt, still gray. ≤30 %: amber, % text Reveals in. ≤15 %: red, one onset nudge, decision-slot escalation. Watts and time-to-empty in the popover. 30 s poll (existing), class changes on threshold edges only.
- **The board**: four fixed bays T1-T4, identical on every output, ≥16 px drawn per bay with ≥24 px hit targets (a11y minimum; the 38 px bar leaves no excuse). Position encodes task; each bay draws its numeral; hue reinforces. This replaces the per-output task pill: the bay whose task is on this output's visible workspace gets a hairline ring in its hue and a static width step (no animation on workspace switch, owner and maintainer veto; hopping is the highest-frequency event in the workflow). Accent ripple stamping moves here, unchanged in effect.

  | state | encoding | motion |
  |---|---|---|
  | no session | hollow socket, numeral 20 % gray | none |
  | working | filled block, numeral mid-gray; bottom 2 px hairline = `N/M` fraction (drawn only when the task has exactly one session) | none; fraction sweeps MOVE_MS on change |
  | waiting, unacked | task hue, full luminance; age chip (`12m`) Reveals at 2 min | one onset nudge, then static |
  | waiting, unacked ≥10 min | luminance tier step (static) | none |
  | waiting, acked | task hue, lower luminance, chip stays | 150 ms crossfade on ack |
  | stopped | near-off dot (quietest tier) | none |
  | stale / data invalid | amber hollow OFF-flag | none |

  Onset nudge implementation: `SlideBin::slide_to(-3.0, 100.0)` with a glib timeout scheduling the return leg (`slide_to` has no completion callback, anim.rs:313-335; the two-calls-in-a-row version in all four concepts does not work as written).
- **Clock**: HH:MM, dim, 1/min boundary-aimed (existing). Calendar popover stays backlog.

### The read layer

One popover chassis (BAR_IDEAS spec 2). Board bay click opens the task popover: full description, raw `N/M ETA` text, per-session rows with working/waiting durations (suspend-skewed mtimes flagged as approximate), last assistant message (requires the nixos-side `last-<pid>` Stop hook; degrades to description without it). Click or Enter on a row focuses that session's workspace via the same path the keybinding takes. This absorbs claude-dash's two unique leftovers; dash retirement is *gated on* the two nixos-side hooks landing, and is a milestone, never a headline (maintainer finding 4). Battery, media, and hazard glyphs open the same chassis with their sections. Everything in every popover is also reachable without hover timing.

### Stand-down table (P10, normative)

| signal | onset | rest | clears when |
|---|---|---|---|
| session waiting | bay nudge + hue step; decision slot fills | static bright; chip at 2 min; tier step at 10 min | any output focuses that session's workspace (pid→workspace map; falls back to task-level). Ack drops luminance; state stays "waiting" until the hook writes working/stopped |
| battery critical | nudge + red | red static + slot occupancy | charger connect edge |
| battery warn | % Reveals, amber | amber static | >30 % or charging |
| urgent workspace | one nudge on its button | red static | workspace focused |
| hazard: caffeine / mode / failed units | glyph Reveals | amber static | inhibit released / default mode event / count 0 |
| stale (OFF-flag) | none (quiet by design) | amber hollow | valid status write |
| OSD | interjection | n/a | 1.5 s decay (self) |

Stop-notification policy (O2): suppress for the session on a visible workspace, deliver with task number + hue attribution for background sessions; policy lives in the in-process NotificationStore (one place), fed by a task hint from the nixos-side hook.

### Cadence budget (P7, normative)

Board + ribbons + decision slot: file/sway transitions only. Waiting-age: 1/min boundary-aimed, alive only while a session waits. Battery: 30 s. Clock: 1/min. Media: player state change; 1/s only while its popover is open. Hazards: D-Bus signals and in-process events. Sub-threshold changes: 150 ms crossfade. Nothing else ticks; any new poll needs written justification.

## Build sequence

Each increment ships independently and leaves the bar better than it found it.

**1. Trust floor (S).** `Activity` gains `Stale` (task.rs:36-61): parse maps unknown values to `Stale`, and the missing-file default at task.rs:221-222 (`map_or(Activity::Waiting, …)`) becomes `Stale`. `css_class()` returns `"stale"`; style.css renders `.bar-task-dot.stale` as an amber hollow ring (transparent fill, 1 px `@warning` border) and deletes the perpetual `.working` pulse keyframes. Update the `activity_defaults_to_waiting` test to assert the new default. No layout change, no new plumbing. This ships before anything else: every later signal inherits its credibility.

**2. TaskStateService (M, ~120 LoC + consumer port).** New `src/task_state.rs` beside SwayService. Moves `claude_pids`, `first_line`, `proc_comm`, `parent_pid`, `window_workspace`, `is_claude_comm` out of task.rs. One GFileMonitor on `~/.local/state/claude-tasks`; each FS event or SwayService change produces one scan into a comparable snapshot: per task 1-4, `Vec<SessionState { pid, desc, activity, progress: Option<(u32,u32)> + raw text, workspace, status_mtime }>`, plus `manual-t<N>`. `connect_change` observer registration mirroring SwayService. A 1/min boundary-aimed glib timeout, created only while any session is `Waiting` and cancelled otherwise, fires change for age-dependent renders. The existing pill becomes a consumer (its `read_view` scan deletes; per-output task selection and render stay), removing today's duplicate per-output /proc walks. Unit tests move with the helpers.

**3. The board (L).** New `src/bar/board.rs` replacing the task pill segment in the right track (task.rs's rendering deletes in the same change; `run_task_command` and click bindings move over; `apply_accent` moves here unchanged). Four fixed bay buttons, min-width 24 px, each containing: numeral label, state styling per the table above via CSS classes (`socket`/`working`/`waiting`/`unacked`/`overdue`/`stopped`/`stale`/`local`), and a 2 px bottom DrawingArea for the `N/M` fill (fraction eased over MOVE_MS with `ease_out_cubic` tick, drawn only for single-session tasks). Transition detection by per-bay snapshot diff (the PillView cache pattern, task.rs:114-129). Onset: the SlideBin nudge with timeout-chained return leg, gated on `animations_enabled()`. Age chip: GtkRevealer, threshold 2 min, text from `status_mtime`. Focus-ack: on SwayService change, any focused workspace resolving to a waiting session's workspace (or its task) drops `.unacked` on that bay across all outputs (stated policy: one owner, ack everywhere). Local bay: `.local` class, hue ring, static width step. Reduced motion: everything jumps; all states are statically distinct.

**4. Decision slot (M).** Center CenterBox occupant behind a GtkRevealer: priority mux over battery state + TaskStateService (occupants 1-2), 40-char description, age chip, handover = outgoing collapses 200 ms then incoming reveals. Media pill deletes; media mark (dim ♪ + popover) lands in the right track.

**5. OSD interjection (M).** BAR_IDEAS spec 5 mechanics routed to the decision slot: `show_display` gains the bar route (needs the `fullscreen` flag on the focused-window snapshot, ~25 LoC sway_ipc), eased continuous hairline across repeated presses, 1.5 s decay, yields back to the standing occupant. Center-screen card remains for fullscreen and lock.

**6. Instrument quieting (S).** Battery threshold tiers + resting dim glyph; clock and start-button luminance drop; tray NeedsAttention filter (SNI `Status` in the tray service). Mostly CSS plus one tray predicate.

**7. Hazard lane (S, then M).** Caffeine glyph from in-process idle-inhibit state and sway `mode` event subscription first; failed-units zbus subscription later or never (severable).

**8. Task popover (M).** Spec-2 chassis on board bay click: rows, durations, click-to-focus. Ships before the hooks; last-message row appears when the hook lands.

**9. nixos-side hooks (S, other repo).** `last-<pid>` Stop-hook write + task-number hint on notify-send; then the NotificationStore suppression policy (S here). Only after both: evaluate claude-dash retirement.

**10. Ribbons (S).** Task ribbons under workspace groups from TaskStateService (low-churn cache key addition). Occupancy ticks stay backlog behind an incremental `rebuild()`.

## Explicitly rejected

- **16-slot app-glyph map replacing `1¹`..`4⁴` labels** (D): vetoed by all four panels; 10 px glyphs fail legibility and the labels are fifteen years of muscle memory in print.
- **Dwell-to-reveal as the only path to detail** (A's ledger/battery/tray trio): the owner never hovers, a11y calls timing-gated information disqualifying, and A itself predicted the degradation to "pretty gray dots".
- **Notification chips and media-handoff toasts in the center** (C): popups with better seating; ~15 track changes an hour with music on; GNOME's centralization lesson not re-derived inside a bar widget.
- **Five-occupant priority ladder with decay timers** (C): a state machine maintained forever, driven by unvalidated policy. Two standing occupants plus one transient is the whole mux.
- **Expand/collapse animation on workspace switch** (B): animates the single highest-frequency event; the in-flight guard means it rarely runs anyway. Static width step instead.
- **1/s playback underline and any ambient playback progress** (B, BAR_IDEAS 8's ambient half): continuous motion for a zero-action state.
- **Continuous 2 s luminance breath as a rest state** (all four concepts): CSS keyframe loops ride the frame clock; hours of per-frame wakeups from a bar whose pitch is stillness.
- **Indeterminate working arc** (BAR_IDEAS 6a): P2 violation, dropped even as backlog.
- **Battery as a 6 px hairline at nominal** (A): owner veto; a dim glyph reads without a pointer.
- **Hue as the sole identity channel in compact bays** (B, C): converges at small sizes and fails deuteranopia; numeral + position are primary.
- **Popover ring buffers** (D's watts sparkline, 30-min transition timeline): permanent memory and code for a surface opened twice a day.
- **Unknown status as Waiting** (current task.rs:46-51): the cry-wolf trainer; replaced by the OFF-flag, increment 1.
- **Tray dwell-dot** (A, B): NeedsAttention filtering gives the same silence with no hidden hover surface.

## Owner decisions (2026-08-02)

The four open questions were put to the owner; all resolved:

1. **Escalation at 10 min unacked**: static luminance step only — nothing on the bar ever loops.
2. **Stopped sessions**: near-off dot in the bay ("slot occupied, session done").
3. **Media mark**: keep the persistent dim ♪ (dims further paused, hidden when no player).
4. **Budget-pacing hazard glyph (O6)**: rejected — "Claude feels slow" gets a shrug, not a mark.
