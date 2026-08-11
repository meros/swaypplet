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

### 3. Screenshot and annotate (M) — DONE 2026-08-11

`src/screenshot/`, four modules: `capture` (pixels), `select` (region),
`annotate` (marks), `deliver` (file, clipboard, card).

Capture is `ext-image-copy-capture-v1` rather than wlr-screencopy — the same
protocol the window switcher (item 5) needs, so there is one capture path in
the process instead of two. `zwlr_screencopy_manager_v1` is still advertised
and deliberately unused.

The selector freezes first and selects second, which is the part slurp
structurally cannot do: it draws on the live screen, so a notification
arriving between the drag and the capture lands in the file. Freezing also
made the colour picker free — the pixel under the pointer is already in hand —
so `hyprpicker`, a Hyprland tool carried for one button, is gone too. The
selector surface is the first one to want no frost, which it gets by being
absent from `layer_effects` rather than disabling blur at map.

The follow-up is a notification, not a new surface: `NotificationStore` already
draws a picture, lays out actions, and treats a wide image as a screenshot.
Annotate / Open / Delete hang off that card, and one gesture now produces both
a file and a clipboard entry.

Annotation has four tools. Pixelate is there because a screenshot of a terminal
is how a token gets shared by accident, and it averages whole blocks rather
than blurring — a Gaussian is invertible enough that text has been recovered
from one.

Absorbed `grim`, `slurp` and `hyprpicker`, plus the `wl-copy` that piped
between them. `wf-recorder` is still installed and still invoked by nothing:
recording is a different feature with a different surface, not a flag on this
one.

Verified headlessly: `dev/render.sh --mode screenshot` (add
`SWPP_SELECT_RECT=x,y,w,h` for the selection chrome) and
`--mode preview:annotate`.

### 4. Privacy indicators (S) — microphone DONE 2026-08-11, the rest blocked

The microphone glyph is in the hazard lane, fed by `crate::audio`'s snapshot:
a source output exists, so something is recording. No timer, and its
stand-down (P10) is that list going empty. The tooltip names what is
listening, because that is the question a microphone glyph provokes.

Camera and screencast were meant to ship beside it and cannot, for the same
reason as each other: **there is no signal a third party can read.** v4l2 has
no in-use broadcast; `/proc/*/fd` scanning would be a poll and needs to walk
every process. The portal is no better — `org.freedesktop.portal.Camera`
exposes `IsCameraPresent` (presence, not use) and `ScreenCast` exposes methods
to *start* a cast with no way to enumerate live ones. Both conditions are
plainly visible in PipeWire's node graph, which this process cannot reach
while libpipewire is out of the build (item 7). They are blocked on that, not
on design, and they become nearly free the day it is in.

### 5. Window switcher with live thumbnails (L) — DONE 2026-08-11

`src/switcher/`, bound to `⌘ Tab`. A grid of every window with its own pixels
on it, arrow keys and typeahead to narrow, Enter to focus.

The join turned out to be free. `ext-foreign-toplevel-list-v1` gives each
window an opaque `identifier`, and sway puts that same string in its tree as
`foreign_toplevel_identifier` — so the model comes from sway IPC (workspace,
focus order, and the `[con_id=N] focus` that actually switches) and the pixels
from `ext-image-copy-capture-v1`, with no title matching between them. The
capture path is the screenshot module's, generalised from an output source to
any capture source.

Order is sway's own: each container's `focus` array lists its children most
recently focused first, so a depth-first walk following it yields session MRU
without swaypplet keeping a history.

Two findings from running it against the live session (11 windows):

- Windows on **invisible** workspaces capture fine — swayfx renders them. The
  worry that a switcher could only show what is already on screen was
  unfounded, which is what makes the surface worth having.
- Spotify failed every time with `buffer_constraints` until the capture
  learned to rebuild its buffer once and retry. The compositor re-sends the
  constraints with that failure; the first implementation treated every
  failure as terminal. 11 of 11 now.

Captures are bounded (`CAPTURE_TIMEOUT`, 400 ms) because the protocol lets a
compositor wait indefinitely for content that never changes; a thumbnail that
does not arrive leaves its card showing the app icon.

The fractional-scale caveat (swaywm/sway#9113) never came up: eDP-1 is at
integer scale 2.0.

Still backlog: the workspace overview this machinery also enables.

### 6. Night light and display profiles in-process (S each)

`zwlr_gamma_control_manager_v1` retires gammastep (20 lines of config, one
systemd unit). `zwlr_output_manager_v1` retires kanshi (63 lines), and
`widgets/display.rs` is already the surface it would hang off. Two daemons out
of the session for very little code, and night-light temperature becomes a
panel control rather than a rebuild.

### 7. zbus and PipeWire, replacing text scraping (M) — audio DONE 2026-08-11

Three migrations, not one. Audio has landed; NetworkManager and BlueZ have not.

**Audio (done).** `src/audio.rs` holds one connection to the sound server and
pushes snapshots into an `Observed`, the same shape `sway_ipc` and `clipboard`
use. Gone with it: the `wpctl status` parser (indentation depth, box-drawing
characters, an asterisk for the default), a second `wpctl` call per device for
its volume, and — the part worth naming — a **2-second poll** that existed only
so plugging in headphones would eventually be noticed. The server had an event
for that all along.

Not libpipewire, and the reason is a build collision rather than a judgement:
the `pipewire` crate generates bindings with bindgen 0.72, this binary already
links `pam-sys` on bindgen 0.69, and cargo's unification of the shared
`clang-sys` leaves the older one unable to load libclang. PipeWire's own
PulseAudio server speaks a protocol with a mature binding and no bindgen at
all. The dependency on `services.pipewire.pulse.enable` is real and is now
stated in `audio.nix` rather than assumed.

The OSD's volume keys stopped spawning anything: the level is computed from the
snapshot on the GTK thread and drawn immediately, which also means the OSD and
the panel slider can no longer disagree.

Source outputs come free, which is what item 4 was waiting for:
`AudioState::microphone_in_use` is a list being non-empty.

**Still to do:** `nmcli` (17 call sites) and `bluetoothctl` (12) over zbus.
Both are read-heavy and their mutating paths (connect to an SSID, toggle the
radio, pair a device) cannot be verified without disrupting the session they
run in, which is the main thing to plan for.

### 8. claude-dash retirement (S, mostly other repo)

An Electron process in the session. BAR_VISION increment 9 gated retirement on
the `last-<pid>` Stop hook plus a task-number hint on notify-send. The hook is
landing and `popover.rs:172` already renders the row, so only the notification
hint is left.

### 9. Keybinds overlay as a real surface (S) — DONE 2026-08-11

Was a `foot` terminal running `cat` on a hand-curated text file, shown and
hidden through a sway window rule and two transient systemd units. Now
`src/keybinds.rs`: a centered glass layer surface whose rows come from sway's
own `GET_CONFIG`, so the sheet cannot disagree with the bindings.

The curated file had in fact drifted. It advertised four screens per task
(`1234 / qwer / asdf / zxcv`) against a grid that has been two since screens c
and d were retired, and called `⌘ Space` the launcher when it opens the control
centre. Both were wrong on the sheet the moment the binding changed; neither is
expressible now.

Presentation is the only hand-written part: a section per command family and a
short phrase per command. A command matching no rule still prints, under
`OTHER`, with its store paths shortened — a cheat sheet that silently omits a
binding is worse than none. The 1 s hold moved into the process
(`HOLD_MS`), which is what let both systemd units go; the session keeps only
the libinput watcher that reports the press and release edges.

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
