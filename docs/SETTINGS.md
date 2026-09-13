# Settings

The settings pane is a page in the Helm card (`:set` in the omnibox, or the
gear in the flight deck; a bare `:` lists every prefix). Five tabs,
`src/settings/`:

| tab | edits | sections of `~/.config/swaypplet/settings.json` |
|---|---|---|
| Look | `output * bg` on the compositor; whether the theme takes its colours from it; how much the shell animates | `wallpaper`, `look` |
| Idle & Lock | the idle manager's timers; the night window; walk-away lock; face unlock; what sudo and pkexec may ask for | `idle`, `elevate` |
| Bar | clock format, segments, OSD placement, key steps, volume boost | `bar`, `keys` |
| Alerts | popup linger, corner and depth; quiet hours; what a screenshot becomes | `alerts`, `capture` |
| Glass | the liquid-glass material | `~/.config/swaypplet/glass.json` |

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
- **Tint** (`look.tint`, off | accents | full) is the one setting that is
  not read where it is used. The panel derives a palette from the wallpaper
  (`src/palette.rs`) and writes `$XDG_CACHE_HOME/swaypplet/palette.css`;
  every process reads that file at `theme::load_css` and the long-lived ones
  follow it on `theme::watch`'s tick. The file's presence is the setting —
  turning the tint off deletes it — so a reader needs neither the settings
  nor an IPC round trip to know what colour anything is, which is what keeps
  a 100 ms JPEG decode out of the lock screen's startup. The panel also
  pushes the derived accents at sway's `client.*` window borders, and puts
  the config's own colours back when the tint goes off.
- **Alerts**: a popup reads linger, corner and depth as it is created and
  keeps them (`notifications/popup.rs`); quiet hours is a 30 s tick plus an
  observer (`notifications/quiet.rs`), edge-triggered so a manual DND
  toggle inside the window stands. Capture is read at the moment of the
  shot (`screenshot/deliver.rs`, `screenshot/mod.rs`).
- **Keys** are read per press by the OSD; the panel's volume rail takes
  the ceiling when it refreshes.
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
  (`wallpaper`, `idle`, `bar`, `glass`).
- `SWAYPPLET_SETTINGS_CONFIG=/path/to/defaults.json` points the system
  layer somewhere else, as `SWAYPPLET_GLASS_CONFIG` does for glass.
- `journalctl -t swaypplet-idle -f` shows the re-arm as
  `idle: settings changed — …` followed by `idle: watching N timeouts …`.
  A night-window boundary logs the same line with `night window open` or
  `night window closed` in place of `settings changed`.
