# Prior art: mobile, wearable, TV, car and spatial

**The interaction ideas that reach a desktop shell from phones, watches and
cars come down to five: fixed slots with contextual content, ongoing state as
an object with a lifetime, notifications ranked by what they ask of you,
continuous gestures you can cancel, and a material that gives way to
contrast.** 42 entries. Sources are marked [verified] (read, or from a search
summary) or [memory].

## Landscape

- **The glass generation (2023–2026).** visionOS and then Liquid Glass made
  translucent, refracting material the shared look across Apple's platforms.
  Apple then spent a year making it more opaque (Clear/Tinted in 26.1, a
  slider in iOS 27).
- **Palette from the wallpaper (2021 onward).** Material You takes hue and
  chroma from the image and fixes tone per role. swaypplet's tint already
  follows it.
- **Ongoing activities (2022–2025).** The Dynamic Island, Live Activities and
  Android 16 Live Updates turned "something is in progress" into a system
  object with sizes, a budget and an end.
- **Notification triage (2017–2021).** Android channels, iOS interruption
  levels, Focus and summaries moved control from the app to the user and from
  "now" to "when it fits".
- **The gesture generation (2009–2013).** webOS, N9, BB10, Sailfish and Ubuntu
  Touch invented today's multitasking and edge gestures, then died for
  ecosystem reasons, not UI ones ([firefox-os](firefox-os.md)).

## The 10 best transferable ideas

1. **Shrink into where it lives.** A backgrounded thing visibly travels to
   its resting place ([dynamic-island](dynamic-island.md),
   [webos-cards](webos-cards.md)).
2. **Every ongoing state has a lifetime and an update budget**
   ([live-activities](live-activities.md)).
3. **Fixed regions, contextual content.** Slots never move; context swaps
   what fills them ([carplay-dashboard](carplay-dashboard.md),
   [watchos-complications](watchos-complications.md)).
4. **Contrast is a floor the material gives way to**
   ([liquid-glass-ios26](liquid-glass-ios26.md),
   [visionos-glass-ornaments](visionos-glass-ornaments.md)).
5. **The sender declares urgency, the user caps it**, with a digest as a
   delivery mode
   ([ios-interruption-levels-summary](ios-interruption-levels-summary.md),
   [android-notification-channels](android-notification-channels.md)).
6. **Peek while held.** You look without leaving, and releasing restores
   everything ([bb10-peek](bb10-peek.md)).
7. **Cancelable, input-driven transitions**
   ([predictive-back](predictive-back.md)).
8. **Undo in place of confirm.** The remorse timer
   ([sailfish-covers-pulley](sailfish-covers-pulley.md)).
9. **Text first, verb second in the launcher**
   ([webos-just-type](webos-just-type.md)).
10. **The host renders templates; the source sends data**
    ([android-live-updates](android-live-updates.md),
    [android-automotive](android-automotive.md)).

## 8 instructive failures

1. **Live Tiles.** Motion and counts in the resting surface became noise, and
   Windows 11 removed them
   ([windows-phone-live-tiles](windows-phone-live-tiles.md)).
2. **The Android 12 Internet tile.** Merging toggles to steer behaviour
   doubled the taps, and Google is reversing it
   ([android-12-internet-tile](android-12-internet-tile.md)).
3. **Stage Manager v1.** A window manager that arranged windows for the user
   lost trust, and Apple rebuilt it in iPadOS 26
   ([ipados-stage-manager](ipados-stage-manager.md)).
4. **Liquid Glass at launch.** Material came before legibility, and the
   opacity was walked back over three releases
   ([liquid-glass-ios26](liquid-glass-ios26.md)).
5. **Google Now.** Uneven predictions taught users to ignore the cards, and
   the product became a news feed
   ([google-now-cards](google-now-cards.md)).
6. **Ubuntu scopes.** Online aggregation in the shell broke and leaked
   ([ubuntu-touch-scopes](ubuntu-touch-scopes.md)).
7. **Android Slices.** A provider API with no surface to show it
   ([android-slices](android-slices.md)).
8. **BB10 gestures.** Brilliant, but invisible: a mandatory tutorial, and
   still hard to learn ([bb10-peek](bb10-peek.md)).

## Ideas for swaypplet (ranked)

| # | Idea | Size | Source |
|---|---|---|---|
| 1 | Bind a "tinted" glass step (more body, less see-through) to `contrast: high` and `prefers-contrast`, and ship it as a visible choice | S | [liquid-glass-ios26](liquid-glass-ios26.md) |
| 2 | A lifetime and a stand-down for every ongoing bar state (a hard cap, then a static rest), plus an update budget that coalesces noisy producers | S | [live-activities](live-activities.md) |
| 3 | An event-triggered digest ("7 while you were presenting"), with the reason shown on the DND tile ("On: screen shared"). This refines roadmap item 3 | M | [ios-interruption-levels-summary](ios-interruption-levels-summary.md), [ios-focus-modes](ios-focus-modes.md) |
| 4 | Popup dismissal travels into the notification centre, and media into its segment. A concrete first case for roadmap item 1 | M | [dynamic-island](dynamic-island.md) |
| 5 | Peek while held: holding a key shows the notification centre or task board, and releasing restores focus unchanged | S | [bb10-peek](bb10-peek.md) |
| 6 | Undo with a countdown in place of confirmation dialogs ("Cleared 12, undo") | S | [sailfish-covers-pulley](sailfish-covers-pulley.md) |
| 7 | "Mute this kind" on the popup, keyed by (desktop-entry, category) | S | [android-notification-channels](android-notification-channels.md) |
| 8 | Geometric arrow-key focus with a defined default in the panel, launcher and Super+Tab | S | [tvos-focus-engine](tvos-focus-engine.md) |
| 9 | Left/right moves between panel sections once one is open from its bar segment | S | [ubuntu-touch-edges](ubuntu-touch-edges.md) |
| 10 | Tile states on, off and *unavailable*, and one convention: click toggles, the secondary action opens detail | S | [android-quick-settings](android-quick-settings.md) |
| 11 | Show the 2–4 ranked wallpaper hues as swatches, plus one chroma ("character") knob next to the tint level | S | [material-you-dynamic-color](material-you-dynamic-color.md) |
| 12 | Launcher verbs on typed text (note, web, emoji, run) below app results | M | [webos-just-type](webos-just-type.md) |
| 13 | A progress segment template (bar, stops, short text) for builds, downloads and sessions | M | [android-live-updates](android-live-updates.md) |
| 14 | A warm, low-luminance lock palette at night from the night light's sun position | S | [standby-mode](standby-mode.md) |
| 15 | Super+Tab and the overview follow the touchpad gesture's progress and cancel on reversal | L | [predictive-back](predictive-back.md) |

**Confirmed, not new:** fixed slots
([carplay-dashboard](carplay-dashboard.md)), no motion at rest
([windows-phone-live-tiles](windows-phone-live-tiles.md),
[kindle-eink-refresh](kindle-eink-refresh.md)), hue-only wallpaper tint
([material-you-dynamic-color](material-you-dynamic-color.md)), and no
network aggregation or prediction in chrome
([ubuntu-touch-scopes](ubuntu-touch-scopes.md),
[google-now-cards](google-now-cards.md)).
