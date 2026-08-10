# Shell ideas — what else belongs in swaypplet

> Research pass 2026-08-10, after the bar vision shipped end to end (BAR_VISION
> increments 1-8 are in `src/bar/`). BAR_IDEAS asked what a *bar* should do.
> This asks what the *shell* should do, and the answer came mostly from two
> places: an audit of what the session still runs outside swaypplet, and a dump
> of the Wayland globals swayfx actually advertises.
>
> Grounded in the same constraints as everything else here: one process owns
> bar + panel + OSD + notifications, motion goes through `anim::Reveal`, and
> nothing ticks without written justification (BAR_VISION P7).

## What the compositor already offers

`wayland-info` against the live session, filtered to the globals that unlock
something swaypplet does not do today:

| global | unlocks |
|---|---|
| `ext_data_control_manager_v1`, `zwlr_data_control_manager_v1` | clipboard history in-process |
| `ext_foreign_toplevel_image_capture_source_manager_v1` | per-window live capture |
| `ext_image_copy_capture_manager_v1` | the buffer side of the same |
| `ext_foreign_toplevel_list_v1`, `zwlr_foreign_toplevel_manager_v1` | window list with titles, app ids, state |
| `ext_workspace_manager_v1` | workspace state without sway IPC |
| `zwlr_gamma_control_manager_v1` | night light |
| `zwlr_output_manager_v1` | display profiles |
| `zwlr_output_power_manager_v1` | per-output DPMS |
| `zwp_input_method_manager_v2`, `zwp_virtual_keyboard_manager_v1` | a picker that *types* instead of copying |
| `zwlr_screencopy_manager_v1`, `zwlr_export_dmabuf_manager_v1` | screenshots and recording |
| `wp_security_context_manager_v1` | sandboxed helper clients |

Notably absent: `xdg_toplevel_icon_manager_v1`. Window icons keep going through
`icons.rs` desktop-file lookup.

## Backlog

Ordered by value per unit of work. Sizes are S/M/L on the BAR_VISION scale.

### 1. Clipboard history, natively (S) — DONE 2026-08-10

`widgets/clipboard.rs` shelled out to `cliphist list`, but nothing in the nixos
config ever ran `wl-paste --watch cliphist store`. `~/.cache/cliphist/db` was
last written 28 May; the panel section had been showing three-month-old entries
since. The bug and the dependency both go away with `ext-data-control-v1`:
swaypplet owns the selection watch, keeps its own ring, and sets the selection
back through its own data source.

Shipped as `src/clipboard.rs`. Notes for later: history is in-memory only, so a
panel restart clears it (deliberate for a clipboard, revisit only with a reason);
text mimes only, images are a follow-up; `x-kde-passwordManagerHint: secret`
offers are skipped, which is the cross-desktop convention password managers
already emit.

### 2. `last-<pid>` PID reuse (S) — DONE 2026-08-10

`popover.rs` keyed the last-message row purely on PID, and nothing ever swept
`~/.local/state/claude-tasks/`. 62 of 64 files belonged to dead processes, so a
recycled PID served a dead session's last message as if it were live. Fixed by
validating the file's mtime against the process start time, computed from
`/proc/<pid>/stat` field 22 plus `btime` and `_SC_CLK_TCK`.

No sweep: the files belong to the nixos-side hooks that write them, and a
reader deleting them is the wrong owner. If the accumulation ever matters, the
Stop hook should clean up after itself. The correctness bug is closed either
way, and a few thousand tiny files a year is untidy rather than harmful.

### 3. Screenshot and annotate (M)

The single largest daily friction. `panel.rs:574` spawns `grim -g "$(slurp)"`
and forgets it: no shutter feedback, no copy-and-save in one gesture, no
annotation, no way to see what was captured. Every primitive is already here —
layer-shell for the region selector, GTK4 snapshot drawing for annotation,
`NotificationStore` actions for the Open/Copy/Delete follow-up.

Absorbs three packages: `grim`, `slurp`, and `hyprpicker` (carried for one
colour-pick button, and a Hyprland tool at that). Gives `wf-recorder` a reason
to exist — it is installed today and invoked by nothing.

Design note: the region selector is a fullscreen layer surface with keyboard
exclusive, which is exactly the dmenu chassis. The frost must be off for it
(the point is seeing the screen), so it is the first surface that wants
`layer_effects` blur disabled at map rather than enabled.

### 4. Privacy indicators (S)

Nothing on wlroots ships them. Mic-in-use, camera-in-use and screencast-active
are three glyphs in the hazard lane (`bar/hazards.rs`), which already has the
appear-only Revealer pattern and the amber static rest state. Mic and camera
come free with the PipeWire migration (item 7); screencast comes from the
portal's D-Bus session list.

Fits P9 and P10 cleanly: each has an obvious stand-down (capture stops), and
none of them is red.

### 5. Window switcher with live thumbnails (L)

sway has no switcher at all. `ext_foreign_toplevel_list_v1` gives the list,
`ext_foreign_toplevel_image_capture_source_manager_v1` plus
`ext_image_copy_capture_manager_v1` give live per-window frames. `wlr-shot` is
the Rust reference for the capture path.

Caveat: per-toplevel capture has a known fractional-scale defect
(swaywm/sway#9113). eDP-1 runs integer scale 2.0, so it misses this machine.

The same machinery is a workspace overview, which is the more interesting
surface: the board encodes task state abstractly, an overview would show it.

### 6. Night light and display profiles in-process (S each)

`zwlr_gamma_control_manager_v1` retires gammastep (20 lines of config, one
systemd unit). `zwlr_output_manager_v1` retires kanshi (63 lines), and
`widgets/display.rs` is already the surface it would hang off. Two daemons out
of the session for very little code, and night-light temperature becomes a
panel control rather than a rebuild.

### 7. zbus and PipeWire, replacing text scraping (M)

13 external binaries, and the three heaviest are parsed as text: `nmcli` (17
call sites), `bluetoothctl` (12), and `wpctl status` through a hand-written
parser (6). zbus is already a dependency, so NetworkManager and BlueZ become
property-change signals instead of poll-and-parse. Native PipeWire replaces the
`wpctl` parser and brings per-app volume and mic-in-use with it.

This is the least visible item and the one that removes the most fragile code.

### 8. claude-dash retirement (S, mostly other repo)

An Electron process in the session. BAR_VISION increment 9 gated retirement on
the `last-<pid>` Stop hook plus a task-number hint on notify-send. The hook is
landing and `popover.rs:172` already renders the row, so only the notification
hint is left.

### 9. Keybinds overlay as a real surface (S)

Today it is a `foot` terminal running `cat` on a hand-curated text file, shown
and hidden through sway window rules and two systemd timers. As a glass layer
surface it would look like the rest of the shell, and deriving it from sway's
IPC config would delete the "update the matching line in the same commit" tax
that `keybinds-help.nix` opens with.

### 10. Tailscale (S)

Three nodes live on this account. `widgets/network/vpn.rs` covers
NetworkManager VPNs only, and `tailscaled` is its own daemon, so exit-node
state and peer reachability are invisible today.

### 11. Emoji and character picker that types (M)

`zwp_input_method_manager_v2` and `zwp_virtual_keyboard_manager_v1` are both
advertised, so the picker can insert into the focused surface rather than
round-tripping through the clipboard. The dmenu chassis is most of the UI
already.

## Rejected

- **Weather, calendar, agenda.** Each wants a network poll or a per-minute tick
  for a surface opened twice a day. P7.
- **Per-app context segment** (BAR_IDEAS 4). Reshapes the bar on the highest
  frequency event in the workflow; the vision asks for stillness, and window
  focus changes hundreds of times an hour.
- **Clipboard persistence across restarts.** cliphist did this and it is how
  clipboard managers leak passwords to disk. In-memory is the safer default;
  revisit only with an encryption story.
- **Image entries in clipboard history v1.** Real want, but the preview,
  memory cap and eviction policy are a separate design.
