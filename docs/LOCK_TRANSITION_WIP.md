# Lock transition — work in progress

**Goal.** A true cross-fade between the desktop and the lock screen, both
directions, with no screenshots and no bad frames. Not a fade to black, not a
frozen capture of the session: the real desktop, composited live, dissolving
into the real lock screen.

**Status.** Steps 1 and 2 done. `T₀` is 65 ms, inside the 250 ms budget, so
the timing problem that blocked the fade is solved. Step 3 (the client-side
ramp) is next; the compositor half is built and waiting.

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
- [ ] **Step 3 — Client-side fade.** Ramp the lock surface's
      `wp_alpha_modifier_v1` multiplier via the existing `SurfaceAlpha`
      (`src/alpha.rs`), 0→1 on lock and 1→0 on unlock, per the spec in
      `Artifacts` below.
- [ ] **Step 4 — Wire the compositor patch** into the NixOS flake and verify
      the whole transition on hardware.

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
