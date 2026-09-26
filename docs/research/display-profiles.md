# Display profiles in the panel, kanshi retired

**Recommendation: match and apply profiles in the panel process over `zwlr_output_manager_v1` (atomic apply, `failed`/`cancelled` feedback, serials), keep sway IPC for what the protocol lacks (workspace moves, HDR later), store profiles as a `displays` section of `settings.json` with Nix defaults, and size it M (about 1,300 lines), not the S that ROADMAP §2 gives it.**

Research, 2026-09-26. **[src]** verified in source or at the link; **[judgement]** my reasoning, unmeasured.

## 1. What kanshi does today

- **[src]** `~/nixos/users/modules/kanshi.nix`: two profiles. `desk-external` sets `eDP-1` to 2880x1800@60 scale 2 at 2560,149, and `NON 28H2U *` to 3840x2160@60 scale 1.5 at 0,0. `laptop-only` sets `eDP-1` alone at 0,0. There are no `exec` hooks.
- **[src]** Two unit workarounds and a `sway.nix` reload-restart line go away with it.
- **[src]** The desk panel reports serial `0000000000000` (`sway.nix`, `externalOutput`). Exact make/model/serial identity is therefore no better than make+model for this monitor.
- **[src]** kanshi `main.c`: a profile matches when every profile output claims a distinct head and every connected head is claimed. Criteria are `*`, the connector name, or `fnmatch` on `"make model serial"`, with `Unknown` for a missing field. The first matching profile in file order wins, and **the current profile is kept while it still matches**, so a manual change is not undone by the next `done`. `exec` runs on `succeeded`; `cancelled` re-matches.

## 2. The protocol

**[src]** `wlr-output-management-unstable-v1.xml`, v4:

- One `head` per connected output, with `name`, `make`/`model`/`serial_number` (v2), `mode` objects (size, mHz refresh, `preferred`), `enabled`, `current_mode`, `position`, `transform`, `scale`, `adaptive_sync` (v4). Unplug sends `head.finished`; every batch ends in `done(serial)`.
- `create_configuration(serial)` must `enable_head` or `disable_head` **every** head; leaving one out is a protocol error. `set_mode`, `set_custom_mode`, `set_position`, `set_transform`, `set_scale` and `set_adaptive_sync` go on the per-head object.
- `test` or `apply` returns exactly one of `succeeded`, `failed` (the compositor "should revert"), or `cancelled` (stale serial, for example a hotplug in flight: retry with the new serial). `apply` can round the values it is given.
- The protocol carries no HDR, colour profile, bit depth or tearing field.

**[src]** swayfx 0.6 (sway 1.12, wlroots 0.20), `sway/desktop/output.c`:

- `output_manager_apply` turns each head into an `output_config` and runs `apply_output_configs` over sway's stored configs plus the new ones, in one commit. It stores them on success and answers `succeeded` or `failed`. `test` runs the same path with `test_only`.
- `update_output_manager_config` marks a DPMS-off output as still enabled, so the `lid:on` → `output * power off` binding does not look like a head change.
- `restore_workspaces` (`tree/output.c`) moves workspaces to an output as soon as it is enabled, when a `workspace … output` assignment names it. `sway.nix` already assigns `y` to the desk monitor and `p` to `eDP-1`.
- `swaymsg reload` re-reads the `output` lines, which is why kanshi is restarted today. The panel must re-apply on reload; `ipc.rs` already counts `Reload` events.

## 3. Alternatives

| Option | Verdict |
|---|---|
| Sway IPC `output` commands plus `get_outputs` | **[src]** One IPC message with several `output` commands produces one modeset (`ipc-server.c`: `force_modeset` after the batch). The reply reports parsing only, so a modeset that fails is visible only by reading `get_outputs` back. There is no `test` and no serial to guard against a hotplug in flight. It does reach `hdr`, `color_profile` and `render_bit_depth`. **[judgement]** Good for the extras, weak as the primary path. |
| wlr-randr | **[judgement]** A CLI over the same protocol; spawning it buys nothing. [wlr-randr](https://gitlab.freedesktop.org/emersion/wlr-randr) |
| kanshi IPC (`kanshictl switch`) | **[src]** The man page lists `reload` and `switch <profile>`. **[judgement]** Keeps the daemon and a second config language, and the panel would only drive it. It does not retire kanshi. |
| shikane, way-displays | **[judgement, from project docs, not verified here]** Both are daemons on the same protocol: shikane with TOML and regex matching, way-displays with YAML, automatic arrangement and its own lid handling. [shikane](https://gitlab.com/w0lff/shikane), [way-displays](https://github.com/alex-courtis/way-displays) |

## 4. Design

**Matching** **[judgement]**. Each profile output carries one criterion: `{make, model, serial?}` with an optional serial, or `{name}` for a connector. A glob covers the `Unknown` case. Keep kanshi's rules: all heads claimed, first profile in order wins, and the current profile stays while it still matches. Re-match only when the *set of head identities* changes, never on every `done`, because the panel's own applies and the Disable button in `display.rs` also emit `done`. If nothing matches, leave outputs alone.

**Hotplug and lid.** **[src]** Hotplug arrives as `head` or `head.finished` followed by `done`, so no poll is needed. The lid stays with `bindswitch` as it is today: power off, then lock. **[judgement]** Clamshell (external screen only, lid shut) is not a feature today. If it becomes one, add an optional per-profile `lid: open|closed` condition, fed by `bindswitch lid:on exec swaypplet display lid closed`. logind's `LidClosed` ([org.freedesktop.login1](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html)) is the fallback, if it emits `PropertiesChanged`.

**Workspaces** **[judgement]**. Sway's own assignments cover the case this setup has. A per-profile workspace map (IPC `move workspace to output`, then refocus) waits until a profile needs it.

**Fields.** The protocol carries mode (`w`, `h`, `refresh_mhz`, or a custom mode), `position`, `scale`, `transform` (the eight values) and `adaptive_sync`. HDR is **[src]** IPC-only (`output <n> hdr on`); when it is needed, send it as a follow-up command after `succeeded`.

**Settings** (follows SETTINGS.md) **[judgement]**:

```json
"displays": { "profiles": [ { "name": "desk-external",
  "outputs": [ { "match": {"name": "eDP-1"}, "enabled": true,
      "mode": [2880, 1800, 60000], "position": [2560, 149], "scale": 2.0,
      "transform": "normal", "adaptive_sync": false },
    { "match": {"make": "NON", "model": "28H2U"}, "mode": [3840, 2160, 60000],
      "position": [0, 0], "scale": 1.5 } ] } ] }
```

The section replaces the Nix default whole, per the two-layer rule. The first "Save" in the pane therefore copies the Nix profiles into the user file, and later Nix edits do not reach that account until Reset.

**UI** (in `widgets/display.rs`). The Displays section shows the active profile name in its summary. It offers "Save current layout as profile" (from the heads' current state), a list with drag-to-reorder (which sets match priority), per-output scale, position and mode rows, and delete. An edit applies live. **[judgement]** UI edits get a 15 s "Keep this layout?" revert, as GNOME does.

**Migration.** Translate the two profiles by hand into `theme/settings.nix`, since two profiles do not justify a generator. Delete `kanshi.nix`, its import and the `sway.nix` reload line in the same change; `cross-repo-guard.nix` checks the keys.

**Failure handling.** Never send a configuration with zero enabled heads; refuse it client-side. On `failed`, **[src]** sway has not stored anything, so the previous state stands; post a notification that names the profile. On `cancelled`, retry once on the next `done`.

**Performance.** Use one wayland connection on a dedicated thread blocked on the fd, the pattern `idle/wayland.rs` already uses, and send results to GTK over `async_channel`. At rest this costs zero wakeups; a hotplug costs one `done`. Add the `wayland-protocols-wlr` dependency. The night-light work needs it too.

## 5. Test plan

- **Unit** (pure matcher over fixture heads): identity against name, glob `Unknown`, first-wins order, current-profile stickiness, a head nobody claimed, and the zero-enabled guard.
- **Nested headless sway** (`WLR_BACKENDS=headless`, as `dev/filmstrip.sh` does). **[src]** `swaymsg create_output` adds `HEADLESS-N`, and `output HEADLESS-2 unplug` removes it (`commands/create_output.c`, `commands/output/unplug.c`), so plug and unplug can be scripted. Check the result with `get_outputs` (rect, scale, transform). **[judgement]** Headless heads likely report `Unknown` make and model and have no fixed modes, so match them by name and set custom modes; confirm this on the first run.
- **Reload**: `swaymsg reload` must re-apply.
- **Failure**: an impossible custom mode must produce `failed`, the state must be unchanged, and a notification must appear.
- **Race**: `create_output` before `apply` must yield `cancelled`, then a retry.
- **On hardware**, once: dock, undock, lid, resume undocked.

## 6. Steps

1. Add `displays` to `schema.rs`, regenerate the defaults, and add a `swaypplet settings` round trip.
2. Write the matcher as a pure module, with unit tests.
3. Write the wayland thread: bind the manager, keep a head/mode model, handle `done`, apply, and retry on `cancelled`.
4. Wire it to the panel: apply at start, on a change in the head set, and on reload; notify on failure.
5. Run the headless integration script.
6. Build the Displays UI: summary, save, reorder, per-output rows, keep-or-revert.
7. nixos: `settings.nix` profiles, delete kanshi, drop the reload line, `nx-check`.

**Size [judgement]:** matcher 250, wayland client 350, schema and CLI 150, UI 300, tests 300, Nix 40. That is about 1,300 lines: M.

## 7. Risks

- **Single owner.** A panel crash leaves hotplug unanswered until systemd restarts it; kanshi had the same exposure.
- **A bad mode on the only screen.** Mitigated by the zero-enabled guard and keep-or-revert; Ctrl+Alt+F2 remains the last resort.
- **Section-whole override** hides later Nix edits from an account that saved once.
- **Reload** can flash through two layouts (sway's lines, then the panel's).
- **Unstable protocol** (v4): skip `adaptive_sync` below v4.

## Sources

- Protocol: [wlr-output-management-unstable-v1.xml](https://gitlab.freedesktop.org/wlroots/wlr-protocols/-/blob/master/unstable/wlr-output-management-unstable-v1.xml) (read from `wayland-protocols-wlr-0.3.12`)
- swayfx at the pinned rev: [sway/desktop/output.c](https://github.com/wlrfx/swayfx/blob/663cf66f92c2d4c99b9c2c4c79ce3538d37470ac/sway/desktop/output.c), `sway/server.c`, `sway/ipc-server.c`, `sway/tree/output.c`, `sway/commands/create_output.c`
- kanshi 1.8 source and man pages: [main.c](https://gitlab.freedesktop.org/emersion/kanshi/-/blob/master/main.c), kanshi(1), kanshi(5), kanshictl(1)
- [sway-output(5)](https://man.archlinux.org/man/sway-output.5.en)
