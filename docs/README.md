# swaypplet docs

**Start with [ROADMAP.md](ROADMAP.md) for what comes next, and
[design-system.md](design-system.md) for how anything on screen is built.**

## Reference: how it works now

| Document | Covers |
|---|---|
| [design-system.md](design-system.md) | Tokens, modes, the wallpaper tint, glass, contrast, components, enforcement |
| [MOTION.md](MOTION.md) | Motion tokens and what each duration means |
| [SETTINGS.md](SETTINGS.md) | The settings store, its layers and the panes |
| [LAUNCHER.md](LAUNCHER.md) | The launcher: sources, ranking, prefixes and keys |
| [BAR_VISION.md](BAR_VISION.md) | The bar's principles (P1–P10) and its layout; the code cites it throughout |
| [AUTH_CARD.md](AUTH_CARD.md) | The password, fingerprint and face card shared by the lock screen and polkit |
| [LOCK_TRANSITION.md](LOCK_TRANSITION.md) | The cross-fade between desktop and lock screen: built, with what is still unverified |
| [QUALITY.md](QUALITY.md) | Reports, crash reports, the auto-fix runner and the Quality tab; the security rule for a public repo |

## Plans

| Document | Covers |
|---|---|
| [ROADMAP.md](ROADMAP.md) | What is in progress, what is next (ranked), and what was rejected and why |

## Research

| Document | Covers |
|---|---|
| [research/liquid-glass.md](research/liquid-glass.md) | The liquid-glass optics the compositor's shader follows |
| [research/adaptive-glass.md](research/adaptive-glass.md) | Glass that responds to its backdrop, beyond the photochromic term |
| [research/display-profiles.md](research/display-profiles.md) | Display profiles in the process, replacing kanshi |
| [research/passkey-cable.md](research/passkey-cable.md) | Why phone-as-passkey unlock (caBLE) is not built: the investigation |

## Prior art

| Document | Covers |
|---|---|
| [prior-art/README.md](prior-art/README.md) | 332 entries on shells, mobile UI, launchers, Wayland, theming, notifications and motion: what worked, what failed, and what swaypplet takes from it. [INDEX.md](prior-art/INDEX.md) lists every entry |

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

## Tools

- `dev/frame-bench.sh`: frame timing for the transitions, per frame, from
  `SWAYPPLET_FRAME_STATS` (`src/frame_stats.rs`). `--gate` fails over the
  limits; `.githooks/pre-push` runs it (`git config core.hooksPath .githooks`).
- `dev/render.sh`, `dev/render-all.sh`: screenshots in a nested sway.
- `dev/filmstrip.sh`: every frame of one transition, as a contact sheet.
- `dev/autofix/run.sh`: one auto-fix job (docs/QUALITY.md); `--dry-run`
  and `--fixture` print what it would send.

## Rules for this folder

- One list of future work: [ROADMAP.md](ROADMAP.md). A new idea goes there
  with a size and a reason, not into a new `*_IDEAS.md`.
- A document whose plan has shipped either becomes reference (it describes
  what the code does) or moves to `history/`.
- A rejected idea moves to the roadmap's Rejected list with the reason.
