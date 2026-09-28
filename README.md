<h1 align="center">swaypplet</h1>

<p align="center">
  <b>A whole desktop shell for sway, in one Rust program, made of glass.</b><br>
  Bar, launcher, control panel, notifications, lock screen, switcher and settings,<br>
  on one design system that turns from dark to light with the sun.
</p>

<p align="center">
  <a href="LICENSE"><img alt="MIT license" src="https://img.shields.io/badge/license-MIT-3a7d6e"></a>
  <img alt="Rust and GTK 4" src="https://img.shields.io/badge/Rust-GTK%204-b7410e">
  <img alt="swayfx" src="https://img.shields.io/badge/compositor-swayfx-5b5fc7">
</p>

<p align="center">
  <img src="docs/screenshots/hero.webp" alt="The control panel over two terminals, wiping from dark mode to light mode and back" width="100%">
</p>

A sway desktop is usually six small tools, each with its own look and its own config: a bar, a launcher, a notification daemon, a locker, a night-light daemon and an output-profile daemon. swaypplet is one GTK 4 program that replaces them. Every surface reads one settings store, one set of colour, contrast and motion rules, and one glass material that the compositor draws. When the theme changes, everything changes together.

- **Liquid glass, drawn by the compositor.** Refraction through a bevelled slab, frost, dispersion and a Fresnel rim, the same on every card.
- **Dark and light, by the sun.** Automatic mode waits until nobody is looking, then fades the whole shell and your apps together.
- **Nothing to glue together.** One binary, one settings file, defaults from Nix, and `swaypplet settings set` for any setting from a key binding.

## A tour

### The bar

<p align="center"><img src="docs/screenshots/bar.webp" alt="The right end of the bar in dark and light mode" width="560"></p>

The bar is still unless something needs you. On the left are the start button and the workspaces, grouped by the screen they are on. The right end is one segmented track: media, battery, the nightly backup, the light/dark/auto switch and the clock. Hover a workspace to see a live picture of it.

### The control panel is the launcher

Super+Space opens the panel. Type and it searches apps, open windows and every setting; `=` calculates and `>` runs a command, and the results learn what you open. Under the list sit the quick switches (Night Light, No Sleep, No Lock, Do Not Disturb); a timed one folds out a duration picker in place.

Each tile at the top opens a full page in the same card:

<p align="center"><img src="docs/screenshots/network.webp" alt="The network page, half in dark mode and half in light mode" width="100%"></p>

<table>
  <tr>
    <td width="33%"><img src="docs/screenshots/audio.webp" alt="Audio: outputs, per-app volume and inputs"></td>
    <td width="33%"><img src="docs/screenshots/bluetooth.webp" alt="Bluetooth: pairing in place"><br><br><img src="docs/screenshots/power.webp" alt="Power: battery, draw, health and profile"></td>
    <td>
      <b>Network</b>: Wi-Fi with signal, security and band, saved networks, VPNs, Tailscale with an exit node, airplane mode.<br><br>
      <b>Audio</b>: the output picker, Bluetooth headset profiles, per-app volume, the microphone and a level test.<br><br>
      <b>Bluetooth</b>: connect, forget and pair in place, with the pairing code on the card.<br><br>
      <b>Power</b>: charge, draw, battery health, the charge limit and the power profile.
    </td>
  </tr>
</table>

### Super+Tab and the key sheet

<p align="center"><img src="docs/screenshots/switcher.webp" alt="The Super+Tab switcher with live workspace pictures" width="100%"></p>

Super+Tab shows every workspace as a live picture and grows the one you pick to full size. Pin a workspace as a small live view in a corner of another. Hold Super for a sheet of every key binding, generated from the sway config that is loaded right now:

<p align="center"><img src="docs/screenshots/keybinds.webp" alt="The key binding sheet" width="100%"></p>

### Notifications that know when to wait

<table>
  <tr>
    <td width="58%"><img src="docs/screenshots/notifications.webp" alt="Grouped notifications, one of them urgent"></td>
    <td>
      Notifications group per app and wait in a notification centre. Quiet hours hold them at night, and they hold by themselves while you share your screen, mirror an output or run something full screen. When you are back, one card says what arrived.<br><br>
      The volume and brightness keys show a small card, and Caps Lock says what it did:<br><br>
      <img src="docs/screenshots/osd.webp" alt="The Caps Lock card above the bar">
    </td>
  </tr>
</table>

### Lock screen, face unlock and polkit

<p align="center"><img src="docs/screenshots/lock.webp" alt="The lock screen, half in dark mode and half in light mode" width="100%"></p>

The lock screen shows the wallpaper as it is. The clock stands on the wallpaper itself, over a soft shade that darkens only what is behind it. Password, fingerprint and face unlock run at the same time, and whichever finishes first unlocks. The lock cross-fades in from the desktop and out again, and the greeter at login uses the same card.

<table>
  <tr>
    <td width="40%"><img src="docs/screenshots/lock-face.webp" alt="Face unlock looking for you"></td>
    <td><img src="docs/screenshots/polkit.webp" alt="The polkit authentication card"><br><br>The polkit agent asks for administrator rights on the same card, with the fingerprint reader as an alternative to the password.</td>
  </tr>
</table>

### Screenshots and recording

Region, window, screen, a colour picker and screen recording, from one selector. The annotation editor crops, draws arrows, boxes and highlights, and pixelates what should not be shared, before you save or copy.

### Settings, inside the shell

<p align="center"><img src="docs/screenshots/glass.webp" alt="The Glass settings tab, half in dark mode and half in light mode" width="100%"></p>

Settings opens in place of the panel, over whatever you were doing. Ten tabs (Appearance, Glass, Idle & Lock, Bar, Input, Alerts, Launcher, Displays, System, Quality), and the launcher searches every setting in them. Nix provides the defaults, and `swaypplet settings get|set` changes any setting from a script or a key binding.

- **Glass** tunes the material: presets, clarity, frost and the bevel, one tuning that means the same in both modes.
- **Displays** saves output layouts as profiles and applies one by itself when its screens connect.
- **Quality** lists the open issues. Super+Alt+Print files a problem with a screenshot and the recent log, and a crash files one by itself. Auto-fix has an agent on this machine reproduce the problem in a nested session, fix it and open a pull request with before and after pictures.

<p align="center"><img src="docs/screenshots/displays.webp" alt="The Displays settings tab" width="100%"></p>

<sub>Every picture here comes from the project's own harness (`SWPP_LOOK=live dev/render.sh`): a nested headless swayfx with the look of a real sway config, over a generated gradient, at scale 2. Devices, networks and notifications are invented test data; the app list and the key bindings come from the machine that rendered them.</sub>

## Why one program

Most of what swaypplet does, a separate tool can do too. What one program adds is agreement: everything on screen comes from the same numbers at the same moment.

- **One theme, one answer.** The panel resolves the theme (dark or light by the sun, the accent, contrast, the wallpaper's hues) and publishes it once. The lock screen, the polkit agent and every other process draw that answer, and the compositor gets the matching glass from it. The text on a card cannot disagree with the glass behind it.
- **A switch never happens in front of you.** Automatic mode waits for the lock, for idle or for ten minutes with nothing open, then fades everything over one motion token. Apps follow the mode too.
- **The look is generated.** Colour scales come from a few inputs in OKLCH and are emitted as CSS tokens. Components use semantic tokens only, and a lint test enforces it.
- **Contrast is proven.** Every text colour is checked with APCA over the glass as the compositor draws it: a model of the shader, over white, grey and black backdrops, for every combination of inputs. The same model holds the two modes to about the same colour from what is behind the glass, so dark and light feel alike.
- **Motion has meaning.** Seven durations (state, expand, enter, exit, move, travel, page), each for one kind of change, all scaled by one setting and by reduced motion.
- **Smoothness is measured.** `dev/frame-bench.sh --gate` times every frame of the panel and the notifications in a nested session, and the pre-push hook fails a push that drops frames.
- **The shell reports its own bugs.** A crash files an issue with its log. A report takes a screenshot. The Quality tab can have an agent fix one in a nested session and show you the pictures before it merges.

The rules behind this are in [docs/design-system.md](docs/design-system.md).

## Requirements

- **swayfx with patches.** swaypplet draws its surfaces as GTK 4 layer-shell windows and asks the compositor for the glass. The liquid-glass material, the fill key, the workspace transitions and the lock screen's cross-fade come from patches to swayfx and scenefx that are not published yet. Without them the shell runs, but the cards are not glass.
- GTK 4.12 or newer, gtk4-layer-shell, gtk4-session-lock, PAM, PipeWire with its PulseAudio server, NetworkManager, BlueZ, UPower and logind.
- Optional: [elephant](https://github.com/abenz1267/elephant) as the launcher's search backend, fprintd for fingerprint unlock, an IR camera and a `howdy-verify <user>` helper for face unlock, and `gh`, `jq` and the [Claude Code](https://claude.com/claude-code) CLI for reports and auto-fix.

## Build and run

With Nix:

```sh
nix build            # the package, in ./result
nix develop          # a shell with the toolchain and the libraries
```

With Cargo, inside `nix develop` or with the libraries above installed:

```sh
cargo build --release
cargo test --release
```

`swaypplet` with no arguments starts the shell (bar, panel, notifications, OSD). It is meant to run as a systemd user service in a sway session. Other entry points are subcommands of the same binary:

| Command | What it does |
|---|---|
| `swaypplet launcher` | open the launcher |
| `swaypplet jump` | the Super+Tab switcher |
| `swaypplet screenshot [region\|window\|screen\|pick\|record]` | take a screenshot or a recording |
| `swaypplet pin` | pin the current workspace as a live picture |
| `swaypplet osd …` | volume and brightness keys, with the on-screen display |
| `swaypplet lock` / `idle` / `greet` | the lock screen, the idle manager, the greeter |
| `swaypplet polkit-agent` | the polkit authentication agent |
| `swaypplet settings get\|set …` | read and change settings from a script or a key binding |
| `swaypplet report [region\|window\|screen]` | a screenshot and a description, filed as a public issue ([docs/QUALITY.md](docs/QUALITY.md)) |
| `swaypplet crash-report` | what systemd runs when the service fails: a de-duplicated crash issue |

## Development

- `dev/render.sh --mode <surface>` renders one surface in a nested headless sway and saves a PNG. `SWPP_LOOK=live` takes the look of the running session's sway config, `SWPP_BEFORE` opens windows behind the glass, and `SWPP_SCALE=2` renders at HiDPI. `dev/render-all.sh capture DIR` renders every surface in dark and light over two backdrops.
- `dev/frame-bench.sh` measures frame timing; `--gate` is what `.githooks/pre-push` runs (`git config core.hooksPath .githooks`).
- The harness never touches the running session: it starts its own compositor, its own cache and its own D-Bus session.

[docs/QUALITY.md](docs/QUALITY.md) describes reports, crash reports and the auto-fix runner (`dev/autofix/run.sh`). [docs/README.md](docs/README.md) indexes the rest of the documentation: the roadmap, the motion and settings references, the research notes, and a databank of 332 entries on prior art in desktop shells.

## License

[MIT](LICENSE)
