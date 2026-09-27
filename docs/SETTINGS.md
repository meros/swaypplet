# Settings

The settings pane is a page in the Helm card (`:set` in the omnibox, or the
gear in the flight deck; a bare `:` lists every prefix). Eight tabs,
`src/settings/`:

| tab | edits | sections of `~/.config/swaypplet/settings.json` |
|---|---|---|
| Look | `output * bg` on the compositor; whether the theme takes its colours from it; how much the shell animates; the night light | `wallpaper`, `look`, `night_light` |
| Idle & Lock | the idle manager's timers; the night window; walk-away lock; face unlock; what sudo and pkexec may ask for | `idle`, `elevate` |
| Bar | clock format, segments, OSD placement | `bar` |
| Input | keyboard layouts, layout switch, Caps Lock, key repeat; touchpad and mouse; key steps, volume boost; the connected devices | `input`, `keys` |
| Alerts | popup linger, corner and depth; quiet hours; what a screenshot becomes | `alerts`, `capture` |
| Launcher | which kinds of result a search lists; ranking by use, and forgetting it | `launcher` |
| Displays | where the outputs stand, and each one's mode, scale, rotation and adaptive sync; the profiles | `displays` (the profiles) |
| Glass | the liquid-glass material | `~/.config/swaypplet/glass.json` |

The Displays tab (`displays_pane.rs`) is the one that does not apply
live: a layout can turn the only screen dark. Its arithmetic is pure and
tested (`arrange.rs`): a dragged output lands flush against the nearest
edge of another, snaps its free edge to theirs, never overlaps and never
leaves one stranded, and the layout goes back against 0,0. Apply sends the
draft through `services::displays::configure` (tested, then applied); a
layout that took is on trial for 15 s (`keep.rs`): Keep, or the layout from
before is sent back, and it is sent back at once when the page goes away
(the panel closed, another tab), so disabling the output the panel is on
from here reverts too. A hand layout is no profile; "Save as profile"
stores the layout on screen. The panel's Displays section keeps its
compact profile list (`widgets::display::fill_profiles`, shared).

`data/settings-defaults.json` is every section at the binary's defaults,
generated from the structs by a test (`cargo test -- --ignored
write_settings_defaults`); `the_shipped_defaults_file_matches_the_structs`
fails when it is stale. The nixos repo's `cross-repo-guard.nix` checks
`theme/settings.nix` against it at eval, so a misspelled key there fails
`nx-check` rather than being dropped at runtime. A key that still reaches
the system layer is logged as an error and ignored; one in the user file is
a warning.

Every tab applies live and saves afterwards (800 ms after the last edit).
Every tab has one Reset, which puts the system default back and removes
the section from the user file.

## Two layers, one shape

```
binary defaults  <  /etc/swaypplet/settings.json  <  ~/.config/swaypplet/settings.json
(schema.rs)         (Nix: theme/settings.nix)        (the pane)
```

A section present in the user file replaces the system's section whole. A
section absent means "the system default", so the user file records what
was changed and a fresh account has no file. `store::system()` reads the
system file once per process; `Settings::load()` reads the user file;
`Settings::idle()` and its siblings resolve the section in force.
`schema.rs` holds the types and what can be done to a `Settings` in memory;
`store.rs` holds the files, the layers and the panel's live copy. A pane
edits through `store::edit::<Section>` and `store::reset::<Section>`, so
"back at the default means no override" is written once.

The wallpaper's system default is not in either file: it is the sway
config's own `bg` line, read back over IPC (`wallpaper::system_default`).

Glass keeps its own file because it is an override of a *material* Nix
ships, with a Nix export; see `glass.rs`.

## The CLI

`swaypplet settings` edits the same user file from a keybind or a script.
Every reader follows the file on its own, so nothing is signalled.

```
swaypplet settings                       the settings in force, as JSON
swaypplet settings get idle.lock_after_s
swaypplet settings set bar.osd_in_bar true
swaypplet settings set wallpaper.path ~/Pictures/wallpapers/pluto-4k.jpg
swaypplet settings reset [section]
swaypplet settings nix idle              the section as theme/settings.nix holds it
swaypplet settings apply                 re-apply the saved wallpaper
```

`set` takes the section from what is in force, so a first `set` on a fresh
account does not zero the other fields. A wrong key or a wrong type is an
error and nothing changes. `apply` is what sway.nix runs from `exec_always`,
so a `swaymsg reload` no longer loses the pick. "Copy as Nix" on the Idle
and Bar tabs is `nix <section>` into the clipboard.

## How an edit reaches its reader

- **Bar** rows are read in this process. `store::update` publishes through
  `store::observe`, so the clock (`bar/clock.rs`), the board (`bar/mod.rs`)
  and the OSD route (`app.rs`) follow at once. The panel also stats the
  file once a second (`store::watch`), so a CLI or hand edit lands in the
  live copy without a restart.
- **Look**: the wallpaper is one `output * bg` command over sway IPC;
  motion is read per animation (`anim::duration`), in every process that
  animates, which is why `lock::run` and `bar::run` call `store::init` too.
- **Apps follow** (`look.apps_follow`, on by default): the panel publishes
  the inputs on screen to the GNOME settings the desktop portal serves
  (`src/theme/apps.rs`): the mode as `color-scheme`, the accent as the
  nearest GNOME `accent-color` (by hue, tint included; slate when grey),
  contrast as `high-contrast`, motion off as `enable-animations` false and
  `reduced-motion` true, and `gtk-theme` between `adw-gtk3` and
  `adw-gtk3-dark` (or `Adwaita` and `Adwaita-dark`) for GTK3. It publishes
  from `theme::changed`, so a sun switch that waits for nobody to be looking
  moves the apps in the same step, at startup, and on every settings
  change. A worker thread reads each key and writes only what differs,
  through GSettings, or `dconf write` when the schemas are missing. Off, the
  keys stay as they were. Home-manager's `dconf.settings` writes the same
  keys on every activation, so the nixos side must not set them.
- **Tint** (`look.tint`, off | accents | full) is a token input
  (docs/design-system.md §2.2). The one part of it that is not a setting,
  the wallpaper's hue, is sampled by the panel alone
  (`src/theme/wallpaper.rs`) into
  `$XDG_CACHE_HOME/swaypplet/wallpaper-source`; every process reads that
  one line in `theme::inputs`, and the long-lived ones follow it on
  `theme::watch`'s tick. A bare file read, so the lock screen's startup
  never pays for a 100 ms JPEG decode. The watching process also puts the
  tokens on sway's `client.*` window borders (`src/theme/sway.rs`), so they
  follow the mode and the tint.
- **Display profiles** (`displays`: `profiles`, a list; each has a `name`
  and `outputs`, each output a `match` of `name`, `make`, `model`,
  `serial` as globs, and optionally `enabled`, `mode` `[w, h, mHz]`,
  `position` `[x, y]`, `scale`, `transform` in sway's spelling, and
  `adaptive_sync`) are read in this process by `services::displays`, which
  replaces kanshi over `zwlr_output_manager_v1`. A profile matches when its
  outputs claim every connected output, each a different one; the most
  specific match wins (an exact serial over make and model over a
  connector), the earlier of equals; the profile on screen stays while it
  still matches. It matches again only when the set of connected outputs
  changes, when the profiles change, and after `swaymsg reload`. Every
  configuration is tested before it is applied; one the compositor refuses
  leaves the outputs as they were and posts a notification. An output no
  profile names and the compositor left at scale 1 gets a scale from its
  density (about 110 px per logical inch, in quarter steps). The section is
  replaced whole by the user's copy, so the first "Save current layout"
  copies the Nix profiles into the user file, and later Nix edits do not
  reach that account until Reset. From the command line,
  `swaypplet settings set displays.profiles '<json list>'`.
- **Night light** (`night_light`: `enabled`, `schedule` sun | always,
  `night_k` 1700–6500) is read in this process by `services::gamma`, which
  holds the compositor's gamma tables over `zwlr_gamma_control_v1` in place
  of gammastep. `store::observe` retargets it: a change ramps over a second.
  With `schedule: sun` it asks `theme::sun` once a minute, the same
  elevation and location the automatic mode uses, and warms linearly in
  mired from +3° (day, 6500 K, no control held at all) to −6° (the end of
  civil twilight, `night_k`). The panel's Night Light tile and the display
  section's warmth slider edit this section too.
- **Alerts**: a popup reads linger, corner and depth as it is created and
  keeps them (`notifications/stack.rs`); quiet hours is a 30 s tick plus an
  observer (`services/notifications/quiet.rs`), edge-triggered so a manual DND
  toggle inside the window stands. Capture is read at the moment of the
  shot (`screenshot/deliver.rs`, `screenshot/mod.rs`).
- **Launcher** is read on every query (`launcher::sources`), so a switch
  applies to the next key typed. The ranking history is not a setting: it
  lives in `$XDG_STATE_HOME/swaypplet/launcher-frecency.json`, and Forget
  history deletes it (docs/LAUNCHER.md).
- **Keys** are read per press by the OSD; the panel's volume rail takes
  the ceiling when it refreshes.
- **Input** (`input`: `layouts` as xkb codes, `se`, `us(dvorak)`;
  `layout_switch` and `caps` as xkb options, `grp:win_space_toggle`,
  `caps:escape`; `repeat_delay_ms`, `repeat_rate`; `touchpad_*` and
  `mouse_*` for tap, natural scrolling, speed −1 to 1, `accel_profile`
  adaptive | flat, dwt, `click_method` button_areas | clickfinger | none,
  `scroll_method` two_finger | edge | on_button_down | none) goes to sway
  as `input type:keyboard|touchpad|pointer …` commands
  (`services::input`), in the panel process. Every field is optional, and
  one left `null` is never sent, so the sway config's own `input` blocks
  stay in force for it: the section holds what was changed and nothing
  else. A store observer sends only the knobs that changed (each xkb
  command compiles a keymap); a knob that goes back to `null` costs a
  `swaymsg reload`, the one way to get the config's value back. A thread
  on the IPC socket re-applies the whole section after a device is added
  and after a reload. `layout_switch` and `caps` share sway's one
  `xkb_options` line, so either one set replaces a config `xkb_options`
  line whole. The tab shows the value the devices report (`get_inputs`,
  read when the panel opens) for a row that has no override; the layout
  picker lists xkeyboard-config's `rules/evdev.lst` (`settings/xkb.rs`;
  the package build names the file in `SWAYPPLET_XKB_RULES`).
- **Wallpaper** is one `output * bg` command over sway IPC.
  `wallpaper::apply_saved` replays it at panel start, and
  `swaypplet settings apply` from the config's `exec_always` replays it on
  reload.
- **Elevate** is read by the polkit agent (`swaypplet polkit-agent`, a
  third process) per request: `polkit/mod.rs` calls `store::current()` on
  every `sudo` or `pkexec`, and `store::watch` keeps its live copy following
  the file. `face` is handed to pam_race as the answer to its `begin`, so
  the camera never opens when it is off; `terminal_card` declines the card
  for a terminal `sudo` (pam_race keeps the prompt); `cue` and
  `typing_abandons_face` are read when the camera reports.
- **Idle** is another process (`swaypplet idle`) with no channel to the
  panel. It stats the user file once a second (`idle/mod.rs`,
  `SETTINGS_POLL`) and, when the mtime moves, reloads and hands the wayland
  thread new timeouts, which destroys its `ext_idle_notification` objects
  and creates them again (`idle/wayland.rs`). Zero on a timer is "never":
  no notification is created. The blank duration and the dim level are read
  at fire time and need no re-arm.
- **The night window** is a second, shorter set of the dim, lock and
  screen-off tiers for a time range (`Idle::resolve`). The same once-a-second
  check resolves it against the local clock, so an edit in the pane and the
  window opening arrive by one path and re-arm by one comparison; a boundary
  is at most `SETTINGS_POLL` late. Suspend is not in the window: it is
  battery-only, and cutting an overnight job short is worse than a late
  suspend. Crossing INTO the window while already idle past its shorter lock
  tier locks at once, which is `ext-idle-notify` being correct — the seat has
  been idle longer than the timeout being armed.

## Adding a setting

1. Add the field to the section struct in `schema.rs`, with a
   `#[serde(default …)]` so an older file still loads, and to the section's
   `Default`. Clamp it in the section's `sanitized` if a bad value is worse
   than ugly.
2. `cargo test -- --ignored write_settings_defaults` to regenerate
   `data/settings-defaults.json`.
3. Add the same field, with the same default, to
   `users/modules/theme/settings.nix` in the nixos repo.
4. Add a row to the tab (`*_pane.rs`), using the helpers in `ui.rs` so it
   lines up with the rest.
5. Read it where it matters: through `store::current()` plus
   `store::observe` in the panel process, or through `cfg` in the idle
   loop.

A setting earns a row when it is a matter of taste that a rebuild is too
slow a loop for. What is deliberately not here: the bar's position and
height (the stylesheet is built around bottom, 38 px), the night light's
temperature (gammastep's config), anything the panel already has a section
for, and anything whose bad value is "gone" rather than "ugly": no setting
may leave the machine unlocked or asleep unlocked, which is why the lock
switches only ever remove a way in or add a lock.

## Trying it without a rebuild

- `dev/render.sh --mode preview:settings.idle` renders one tab
  (`wallpaper`, `idle`, `bar`, `input`, `alerts`, `launcher`, `displays`,
  `glass`). `settings.input` sends the saved `input` section to the nested
  compositor and lists its devices.
  `settings.displays` drives the nested compositor's outputs;
  `SWAYPPLET_PREVIEW_DISPLAYS_APPLY=1` moves one and applies it after two
  seconds (the keep question, then the revert), `=leave` also hides the
  page three seconds later.
- `SWAYPPLET_SETTINGS_CONFIG=/path/to/defaults.json` points the system
  layer somewhere else, as `SWAYPPLET_GLASS_CONFIG` does for glass.
- `journalctl -t swaypplet-idle -f` shows the re-arm as
  `idle: settings changed — …` followed by `idle: watching N timeouts …`.
  A night-window boundary logs the same line with `night window open` or
  `night window closed` in place of `settings changed`.
