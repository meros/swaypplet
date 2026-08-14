# Lock transition — work in progress

**Goal.** A true cross-fade between the desktop and the lock screen, both
directions, with no screenshots and no bad frames. Not a fade to black, not a
frozen capture of the session: the real desktop, composited live, dissolving
into the real lock screen.

**Status.** Step 1 done, step 2 next. The compositor half is built and
shelved; the client cannot yet hit the timing window it needs, and fixing that
is the current work.

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
- [ ] **Step 2 — Make the locker resident.** The process starts with the
      session and stays alive across locks. The supervisor tells it to lock
      rather than spawning it. Re-measure `T₀`.
- [ ] **Step 3 — Client-side fade.** Ramp the lock surface's
      `wp_alpha_modifier_v1` multiplier via the existing `SurfaceAlpha`
      (`src/alpha.rs`), 0→1 on lock and 1→0 on unlock, per the spec in
      `Artifacts` below.
- [ ] **Step 4 — Wire the compositor patch** into the NixOS flake and verify
      the whole transition on hardware.

---

## Step 2 design notes (not yet built)

Shape: the supervisor spawns `swaypplet lock` once and keeps it. It writes a
line to the locker's stdin to request a lock; the locker answers on stdout as it
does today. That reuses the existing pipe plumbing in `src/idle/locker.rs`
(which already pipes stdout and reads `LOCKED`) and adds no socket, no new
protocol and no new permissions.

Open questions to settle while building it:

- **The warm window.** Residency only pays off if something is presented
  early: the 881 ms lands on the first window whenever it happens. A 1×1
  transparent layer-shell window presented and destroyed at startup is the
  cheapest way to pay it at login instead of at first lock.
- **What survives an unlock.** GTK init, CSS and the wallpaper texture clearly
  should. The widget trees probably should, re-parented into fresh windows
  each lock, which also keeps GSK's uploads warm. `gtk4-session-lock` creates
  the windows in `connect_monitor` during `lock()` and destroys them at unlock,
  so the content has to be separable from the window.
- **What must not survive.** `AttemptGate` state, any typed password, the face
  and fingerprint workers (currently started on `locked` and reaped by process
  exit), and the `unlocking` latch.
- **Crash handling.** Today the supervisor only watches the locker during a
  lock. A resident process needs restarting if it dies while idle, and the
  existing behaviour must be preserved if it dies while locked (the session
  stays locked, sway shows the abandoned-lock screen).
- **Cost of residency.** A GTK process alive for the whole session. Measure
  RSS; the panel is ~50 MB for comparison.

Security position: the locker already runs as the user and can lock the session
at any time, so keeping it alive grants it nothing it did not have. It holds no
secrets between locks.

---

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
