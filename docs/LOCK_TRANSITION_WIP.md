# Lock transition — work in progress

**Goal.** A true cross-fade between the desktop and the lock screen, both
directions, with no screenshots and no bad frames. Not a fade to black, not a
frozen capture of the session: the real desktop, composited live, dissolving
into the real lock screen.

**Status.** Built, wired, and looked at. `T₀` is 65 ms, the client ramps its
own surface, the patched compositor defers `locked` until the ramp finishes,
and an unpatched one degrades to today's hard cut. The first human look found
the entrance stuttering and the card moving under itself; both are fixed and
measured, see "What looking at it found" below. The exit was right the first
time.

---

## Why this needs a resident locker

The transition is gated on one number: `T₀`, the time from the lock request to
the locker's first painted frame. The compositor holds the desktop visible and
defers `locked` until the lock surface is opaque, so `T₀` is exactly how long
the user stares at an unchanged desktop after pressing Super+L. The design
budgets 250 ms for it before falling back to today's hard cut.

Measured with `stage()` (`src/lock/mod.rs`), nested sway on the **wayland**
backend so GTK used the real GPU and the Vulkan renderer:

```
entry                     0 ms
gtk init                 21 ms
css                      86 ms
monitor 1: built        103 ms      widget tree: 17 ms
monitor 1: assigned     984 ms      <-- 881 ms
monitor 2: built        985 ms
monitor 2: assigned     996 ms      <-- 11 ms
FIRST FRAME painted    1020 ms
```

`T₀ ≈ 1.0 s`, and **881 ms of it is a single call**:
`gtk_session_lock_instance_assign_window_to_monitor`, which ends by calling
`gtk_window_present()` itself (`gtk4-session-lock.c:260-317`).

What it is not, each ruled out by measurement:

- **Not the renderer.** `GSK_RENDERER=vulkan|ngl|cairo` gives 817 / 828 / 835
  ms. Pure software costs the same as Vulkan.
- **Not accessibility.** `GTK_A11Y=none` gives 883 ms, and the a11y bus is
  running in the real session anyway.
- **Not the wallpaper.** The 4K JPEG decode is 41-61 ms. (An earlier bisect
  suggested ~400 ms; that was wrong, `SWAYPPLET_LOCK_WALLPAPER` was unset in
  the measuring shell and the difference was variance.)
- **Not fontconfig.** The cache is warm and `fc-match` is fast.
- **Not waiting on the compositor.** Sampling `/proc/PID/stat` through the gap
  shows `state=R`, ~1.05 CPU-seconds accumulated across threads, `wchan =
  futex_do_wait`. It is genuine multi-threaded computation.

Two facts settle the design:

1. **It is one-time per process.** The second monitor's lock surface costs
   11 ms against the first one's 881 ms.
2. **It is not lock-specific.** Presenting a throwaway 1×1 layer-shell window
   before `instance.lock()` makes both real lock surfaces cost 14 ms and 15 ms
   — but the throwaway window then pays 1774 ms itself. It is generic
   first-GTK-window cost, and it is paid by whoever presents first.

So warming inside the locker only moves the cost; nothing removes it from a
process that is born for one lock. The process has to outlive the lock. With
residency, the first window is presented once at session start and every lock
afterwards costs roughly 15 ms per monitor plus ~25 ms to first frame, i.e.
`T₀ ≈ 50 ms` against a 250 ms budget.

This is worth doing on its own merits regardless of the fade: **the lock screen
currently takes about a second to appear after Super+L.**

## Plan

- [x] **Step 1 — Break down the second.** Done, see above. `stage()` in
      `src/lock/mod.rs` is permanent, debug level. Answer: 881 ms of one-time
      first-window cost, recovered in full by residency.
- [x] **Step 2 — Pre-warm the locker.** Done, and smaller than full residency:
      the process is still one-lock-per-process, so nothing had to become
      re-entrant. It is simply spawned early, absorbs the first-window cost
      while nothing waits, and parks on stdin until told `LOCK <reason>`.
      **`T₀` measured at 65 ms**, from 1000 ms.
- [x] **Step 3 — Client-side fade.** `src/lock/fade.rs`. Multiplier set to 0
      at `::realize` (before the first buffer, so the first presented frame is
      exactly 0), ramp started on the first `after-paint`, and every value set
      as *pending* state with a `queue_draw` rather than committed, because a
      client commit on a lock surface is `null_buffer` or
      `dimensions_mismatch` and both are fatal. Verified degrading cleanly on
      an unpatched compositor: "compositor has no lock_fade; cutting".
- [x] **Step 4 — Wire the compositor patch.** `patches/swayfx-lock-crossfade.patch`
      is in `flake.nix`. Headless smoke test with the fade genuinely armed:
      no protocol errors, `LOCKED` emitted once, the locker still alive 8 s
      after locking, and `session locked` arriving *after* the first painted
      frame, which is the deferral doing its job.
- [x] **Step 5 — Look at it.** Done, and it found two things the headless
      tests could not. See below.

---

## What looking at it found

The exit was smooth on the first try. The entrance stuttered and the card
grew a row halfway through it. Two causes, neither visible to any test that
does not either watch pixels or read the wire.

### The multiplier reached the compositor ten times out of thirty-eight

This is the important one, and it invalidates the obvious instrument.

The ramp value is `wp_alpha_modifier_v1` multiplier state, which is
double-buffered: it reaches the compositor only on the surface's next
`wl_surface.commit`. A lock surface may not be committed by hand
(`null_buffer`, `dimensions_mismatch`, both fatal), so `fade.rs` asks GTK for
the commit with `queue_draw` and rides GTK's own. **GTK produces no frame when
the render node tree is identical to the last one**, and a lock screen at rest
is pixel-identical frame to frame — the more so since `.lock-crossfade
.lock-card { animation: none }` removed the card's entrance.

Measured on the real session with `WAYLAND_DEBUG=1` (`~/scratch/fade-probe.sh`):

```
entrance 356 ms:  38 multipliers set,  11 commits
    143 ms  COMMIT          <- last one for 185 ms
    145 ms  mult 0.220
    ...     0.25 ... 0.97 set, none of it leaving the client
    328 ms  COMMIT          <- screen jumps 0.22 -> 0.97
```

The screen held at a fifth opacity for 185 ms and then snapped to full. The
frame clock, meanwhile, was flawless: `after-paint` fired 20 times at a 16 ms
median, because GTK runs the frame cycle whether or not it draws. **Counting
frames measures nothing here; count commits.**

The fix is `SurfaceSet::pulse` — one pixel in the corner, alternating between
two alphas a 255th apart, invalidated on every broadcast. GSK sees a changed
node, damages it, and commits. Nested headless, same binary, same ramp:

```
before:  10 frames the compositor could act on, ramp visibly stalls at 0.28
after:   23 frames, one every 16.3 ms, 0.000 -> 1.000, biggest step 0.080
damage:  (0,0,1,1) per frame, and nothing else after the card settles
```

The exit never showed the fault because an unlock always follows an auth
success, and `.lock-success`'s 200 ms border transition repaints the card on
every frame. It was smooth by accident.

### The card grew a row mid-fade

`switch_user::list` is two D-Bus round trips (logind, then fprintd). It was
started when the lock command arrived and answered inside the entrance, where
applying it rebuilds every chip, decodes every avatar and re-lays out the
card — all of it seen through a half-transparent surface. The list is now
fetched during the warm-up (13 ms, measured, with nothing waiting on it), so
the chips are in the first painted frame; the lock-time query stays as a
refresh, is held until the entrance ends (`LockFade::on_settled`), and no-ops
when it agrees with what is already on screen.

*Since superseded.* The lock screen has no chip row at all now — a locked
session authenticates one user, so the picker belongs to the greeter, the
surface that asks who you are, and the lock kept the "Switch user" button that
reaches it. Both queries went with the row, warm-up and refresh, and with them
this whole class of fault: nothing arriving from a worker can change the card's
height any more. `LockFade::on_settled` survives, with no caller, for the next
thing that can.

### Two smaller things, same pass

- The entrance used `ease_out_cubic`, which spends its biggest step on the
  first frame of the ramp — the frame right behind the surface's cold first
  paint. That is the argument `start_exit_ramp` already makes for smoothstep;
  the entrance now uses it too.
- The ramp parks at 0 for two refresh periods before moving, which covers the
  double-commit burst visible at +19 and +20 ms while the initial configure
  settles. Invisible, and it costs only `T₀`.

### Tools

- `~/scratch/fade-probe.sh` — locks the real session under `WAYLAND_DEBUG=1`.
- `~/scratch/lockfade-nested.sh` — same, in a nested headless swayfx, so the
  wire can be read without locking anything. Takes a binary path.
- `~/scratch/fade-analyze.py <log>` — pairs `set_multiplier` against the
  commits that promote it and prints the longest stretch with no commit.

---

## Step 2 as built

Full residency was not needed, and the version that shipped is much smaller.
The locker is still **one lock per process**: it is simply born early. Nothing
had to become re-entrant, no worker needed a shutdown path, `AttemptGate` and
the `unlocking` latch are fresh by construction, and every existing exit code
and crash-relaunch rule is untouched.

- `swaypplet lock` with `SWAYPPLET_LOCK_WAIT=1` presents a 1×1 transparent
  background-layer window to absorb the first-window cost, destroys it, prints
  `READY`, and blocks reading stdin until it gets `LOCK <reason>`
  (`src/lock/mod.rs`, `warm_and_wait`).
- The supervisor keeps one armed child in `ARMED` and hands it the reason on
  stdin (`src/idle/locker.rs`, `prewarm` / `take_armed`). The idle manager arms
  one at startup and another after every `LockerGone`.
- The reason moved from the spawn environment to the `LOCK` line, since the
  process now exists before anyone knows why the next lock will happen.
  `crate::lock::reason()` reads either, so a directly-spawned `swaypplet lock`
  still behaves exactly as before.

Measured end to end, nested sway:

```
warm: done            1948 ms      absorbed while idle
lock commanded        3970 ms
  build                 17 ms
  assign                15 ms
  present -> frame      31 ms
FIRST FRAME painted   4035 ms      T₀ = 65 ms
```

Costs and open points:

- **113 MB RSS** for the parked process (`VmRSS` while blocked on stdin). The
  panel is ~50 MB. This is the price of the warm-up and it is paid for the
  whole time the session is unlocked.
- **Fallbacks are all cold, never broken.** No armed child, a child that died
  while parked, or a broken stdin pipe each fall through to spawning the old
  way. A crash-relaunch mid-lock also spawns cold, deliberately: it is the rare
  path and holding a spare for it would mean a second parked process.
- **Untested on hardware.** Everything above is the nested compositor.

Security position: the locker already runs as the user and can lock the session
at any time, so a parked one grants it nothing it did not have. It holds no
secrets before the lock it was spawned for.

---

## How to verify

After `nx-switch` (which restarts the compositor, so the patched swayfx is
only live in a fresh session):

1. Lock with Super+L. The desktop should dissolve into the lock screen over
   ~300 ms rather than cutting. Nothing should flash black, and the lock
   screen must never appear at full opacity and then fade.
2. Unlock. The lock screen should dissolve back into the desktop, and the
   desktop should already be drawn when it becomes visible rather than
   appearing after it.
3. `journalctl --user -t swaypplet-idle -g lock_fade` should be silent. Any
   "compositor has no lock_fade; cutting" means the session is still running
   the old compositor.
4. Close the lid and reopen. This path deliberately does NOT fade
   (`locker.rs` sends `nofade` for `sleep`), so it should cut exactly as it
   does today, and suspend should not be delayed.

If anything misbehaves, in ascending order of blast radius: set
`SWAYPPLET_LOCK_FADE=0` in the locker's environment to disable the client
half; or drop `./patches/swayfx-lock-crossfade.patch` from `flake.nix` and
rebuild, which returns the compositor to stock and makes the client cut on its
own because `lock_fade` stops answering.

## What is NOT verified

- **The midpoint.** The predicted sRGB dip and the exact feel of the curve are
  unmeasured.
- **Multi-monitor.** Tested with one and two headless outputs for the timing
  work, never for the fade itself.
- **A real 4K external.** Frame cost at that resolution is untested.

A note on method, because it cost an afternoon: nested compositors on the
**wayland** backend put a real window on the user's desktop, and a lock screen
there is indistinguishable from the real one. Two "findings" during this work
turned out to be the user authenticating to a nested lock screen. Test
headless, where nothing is visible and nothing invites interaction.

## Artifacts

- **Compositor patch:** `~/nixos/patches/swayfx-lock-crossfade.patch`, 942
  lines, untracked. Applies to swayfx 0.6 (`fd71a6b`), builds clean under
  `-Wall -Wextra -Werror`, smoke-tested headless. Backdrop starts at 1/255
  instead of opaque, steps to opaque when the locker's multiplier reaches 1.0,
  and `locked` is deferred until that frame is presented on every output. Adds
  a `lock_fade on|off <ms>` IPC command, because the backdrop's starting alpha
  has to be chosen in the same dispatch as the lock request and nothing in the
  protocol tells the compositor the locker's intent at that moment. An
  unpatched compositor answers `Unknown/invalid command`, which is the version
  gate: the client then disables the fade and behaves exactly as today.
- **Full design spec:** `scratchpad/spec.md` from the design workflow (11
  agents). Catalogues 44 possible glitches, 24 real with mitigations, 20
  refuted with reasons. Includes frame-by-frame timelines for both directions.
  Not in-tree; regenerate or copy it here if it needs to outlive the scratch
  directory.

## Decisions already taken

- **No screenshot of the desktop.** hyprlock's approach: capture the session
  before locking and cross-fade against the image. Rejected because the locker
  would hold a full-resolution picture of the unlocked session for the whole
  locked period, with no memory hardening, and it delays the lock by the
  capture round trip.
- **No `session_lock_xray`.** Hyprland's escape hatch keeps rendering the
  desktop under a translucent lock surface for the entire lock. Off by default
  there for good reason.
- **`locked` is deferred rather than sent early.** This makes swayfx *more*
  conformant, not less: `ext-session-lock-v1` requires that `locked` not be
  sent until a locked frame has been presented on every output, and stock sway
  sends it synchronously before any lock surface exists (`sway/lock.c:301`).
- **The unlock fade accepts a bounded deviation.** Normal surfaces are
  composited while `locked` is still in force, for ~285 ms, entered only after
  successful authentication and with input still routed to the lock client.
  Revealing the desktop early to someone who just authenticated is not an
  exposure; the blanking requirement exists to stop showing the session to
  someone who has not.

## The card must not move while the fade plays

A cross-fade can carry a card onto the screen. It cannot carry a card that
changes shape halfway through, and the lock card used to change shape twice on
every entrance: the fingerprint pill appeared when fprintd finished claiming
the reader (a few hundred milliseconds in, squarely inside the fade), and the
status and Caps Lock rows appeared and vanished under it afterwards. Each
arrival grew the card downward and re-centred the whole column, so the thing
the fade was ramping up was also sliding around.

The rule now, stated in `docs/AUTH_CARD.md` and kept by `src/auth_field.rs`:
**a card's geometry is decided when it
is built and never changes again.** Anything that can arrive late is laid out
from the first frame and only painted or not — a fingerprint pill in a
reserved row, one message line carrying the status and the Caps Lock warning
between them. Anything that genuinely differs card to card (a greeter's
username row, a polkit identity picker, whether there is a fingerprint reader
on this machine at all) is settled *before* the surface is presented, where a
size change costs nothing.

That last one is why the lock's warm-up asks fprintd whether this user has
enrolled prints (`fp::self_enrolled_blocking`) alongside the user list: the
answer decides whether there is a fingerprint row at all, and it has to be
known before the card exists rather than when the reader reports in.

Verify with the render harness — the card's bounding box must be identical in
every one of these:

    for st in "" fp fp,error fp,face; do
      SWAYPPLET_PREVIEW_LOCK_STATE="$st" dev/render.sh --mode preview:lock --out /tmp/lock-$st.png
    done

`SWAYPPLET_PREVIEW_CAPS=1` adds the Caps Lock line (headless sway has no
keyboard to latch), and `SWAYPPLET_PREVIEW_POLKIT_STATE` does the same job for
`--mode preview:polkit`.
