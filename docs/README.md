# swaypplet docs

**Start with [ROADMAP.md](ROADMAP.md) for what comes next, and
[design-system.md](design-system.md) for how anything on screen is built.**

## Reference: how it works now

| Document | Covers |
|---|---|
| [design-system.md](design-system.md) | Tokens, modes, the wallpaper tint, glass, contrast, components, enforcement |
| [MOTION.md](MOTION.md) | Motion tokens and what each duration means |
| [SETTINGS.md](SETTINGS.md) | The settings store, its layers and the panes |
| [BAR_VISION.md](BAR_VISION.md) | The bar's principles (P1–P10) and its layout; the code cites it throughout |
| [AUTH_CARD.md](AUTH_CARD.md) | The password, fingerprint and face card shared by the lock screen and polkit |
| [LOCK_TRANSITION_WIP.md](LOCK_TRANSITION_WIP.md) | The cross-fade between desktop and lock screen: built, with what is still unverified |
| [PASSKEY_CABLE.md](PASSKEY_CABLE.md) | Why phone-as-passkey unlock (caBLE) is not built: the investigation |

## Plans

| Document | Covers |
|---|---|
| [ROADMAP.md](ROADMAP.md) | What is in progress, what is next (ranked), and what was rejected and why |
| [superpowers/specs/](superpowers/specs/) | Design specs written for single features |

## Research

| Document | Covers |
|---|---|
| [research/liquid-glass.md](research/liquid-glass.md) | The liquid-glass optics the compositor's shader follows |
| [research/adaptive-glass.md](research/adaptive-glass.md) | Glass that responds to its backdrop, beyond the photochromic term |
| [research/display-profiles.md](research/display-profiles.md) | Display profiles in the process, replacing kanshi |

## History

| Document | Covers |
|---|---|
| [history/shell-ideas-2026-08.md](history/shell-ideas-2026-08.md) | The August 2026 shell pass: what shipped and why (clipboard, screenshots, zbus and PipeWire, keybinds) |

## Zoos

Static pages that show components and options side by side. Open them in a
browser.

| Page | Shows |
|---|---|
| [design-system-zoo.html](design-system-zoo.html) | The design system's tokens and components, dark and light |
| [liquid-glass-zoo.html](liquid-glass-zoo.html) | Liquid-glass physics and material variants |
| [auth-zoo.html](auth-zoo.html) | Surface alignment and motion across the auth, lock and polkit surfaces |
| [helm-zoo.html](helm-zoo.html) | Concepts for the panel as a command deck ("helm") |
| [delight-zoo.html](delight-zoo.html) | Micro-gestures: small motion and feedback details |

## Tools

- `dev/frame-bench.sh`: frame timing for the transitions, per frame, from
  `SWAYPPLET_FRAME_STATS` (`src/frame_stats.rs`). `--gate` fails over the
  limits; `.githooks/pre-push` runs it (`git config core.hooksPath .githooks`).
- `dev/render.sh`, `dev/render-all.sh`: screenshots in a nested sway.
- `dev/filmstrip.sh`: every frame of one transition, as a contact sheet.

## Rules for this folder

- One list of future work: [ROADMAP.md](ROADMAP.md). A new idea goes there
  with a size and a reason, not into a new `*_IDEAS.md`.
- A document whose plan has shipped either becomes reference (it describes
  what the code does) or moves to `history/`.
- A rejected idea moves to the roadmap's Rejected list with the reason.
