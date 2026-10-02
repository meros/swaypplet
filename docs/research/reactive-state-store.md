# A reactive state store

**Idea, not scheduled.** Vetted 2026-10-02 in a short read of the code; nothing here is measured.

**Recommendation: do not build a store daemon. Make the in-process `Observed` cell the one typed store per binary, then move one cross-process slice (power: AC and battery) onto a published D-Bus object the way presence already is. Keep lock and unlock authority out of any store.**

## How the parts talk today

- **In-process cells.** `service::Observed<T>` (`src/service.rs:11-56`) is already a small store: a value, `connect_change`, and `set_if_changed` that drops no-op writes. It appears in 18 files. Each service wraps it in its own `thread_local!` and its own `observe(cb)` function (`src/services/battery.rs:35-49`, `src/services/inhibit.rs:243-258`, `src/services/displays/mod.rs:54-66`). Sway is the same shape with its own type (`SwayService::connect_change`, `src/sway/ipc.rs:195`).
- **Settings.** One file, followed by inotify in the panel (`src/settings/store.rs:181-187`) and again in the idle process (`src/idle/mod.rs:194-197`). Nix writes `/etc/swaypplet/{settings,glass,switch-user}.json` as read-only defaults (`store.rs:51`, `settings/glass.rs:43`, `switch_user/host.rs:54`).
- **Files as a bus.** Claude task state is a directory of files written by hooks and watched with a GFileMonitor (`src/services/task_state.rs:1-12`).
- **D-Bus.** The idle process owns the presence sensor and publishes `dev.swaypplet.Presence` (`src/services/presence.rs:338-398`). The bar and the lock screen subscribe. This is the cross-process store pattern, already working, for one key.
- **systemd and subprocesses.** Inhibitors are user units; their state is read by spawning `systemctl` (`src/services/inhibit.rs:134-142`).
- **Across repos.** Eight tables are written twice by hand and checked at eval by nixos `users/modules/cross-repo-guard.nix`.

## Pain points the idea would address

1. **Same fact, two sources.** The idle process decides "on AC" by reading `/sys/class/power_supply/*/online` at suspend time (`src/idle/mod.rs:700-707`, used at `:490`). The bar follows UPower, falling back to a 60 s sysfs poll (`src/services/battery.rs:57-70`). They can disagree, and only one of them is pushed.
2. **State nobody pushes.** Inhibitor cells are fed by "whoever establishes a reading": the panel's tiles on toggle and `inhibit::prime` at startup (`src/bar/hazards.rs:14-18`). A timed No Sleep unit that exits on its own is not published, so the bar glyph can stay until something re-reads it. The idle process skips the cache and spawns `systemctl` (`src/idle/mod.rs:493`).
3. **Coarse change events.** The settings cell notifies on every edit, so each consumer diffs its own slice (`src/services/devices.rs:62-77`). A slider drag is dozens of notifications.
4. **Thread-local traps.** `settings::store::current()` on a worker thread silently returns defaults (`src/settings/store.rs:189-198`). The comment names a bug it already caused.
5. **Belt and braces.** The idle loop rechecks settings every minute on top of inotify (`src/idle/mod.rs:191-192, 329-333`).

## What a store would be

Two different things share the name.

- **In-process, per binary.** A typed key registry over `Observed`: one `thread_local!` table, keys as types, per-key change events, and a `Send` read handle so workers stop reading defaults. This is Elm or Redux without the reducer ceremony. Cheap, and it fixes 3 and 4.
- **Cross-process.** A bus where producers own keys and every process subscribes. Fixes 1 and 2. It does not need a new daemon: D-Bus properties with `PropertiesChanged` already give typed keys, a snapshot on connect, change events and single-writer ownership by bus name. `presence.rs` does exactly this.

Prior art, briefly: Redux and Elm (single store, pure updates); D-Bus properties (what UPower, logind and NetworkManager already speak to us); varlink (typed, socket-based, used by systemd); Home Assistant's state machine (entity ids, `state_changed` events, one owner per entity); PipeWire's `pw-metadata` (keyed, subscribed, cross-process); Hyprland and niri event streams (the compositor as a broadcast source, which sway IPC already is); eww and ags variables (polled or listened values bound to widgets).

## What it costs

- **The lock path.** The idle process and the locker are the safety-critical pair. A store daemon on that path is a new way to strand a locked session. Rule: the store may inform the locker (presence, battery on the lock screen), it may never decide lock, unlock, blank or suspend. An unreachable store must read as "unknown", and policy must fall back to the safe side, as `inhibit.rs` already documents for No Sleep and No Lock.
- **Ordering.** Today each process sees its own sources in their native order. A relay adds a second ordering. Per-key last-writer-wins is enough; cross-key transactions are not worth having.
- **Schema across Nix generations.** The panel, the idle process and a store can run from different generations after a switch until they restart. Keys need versioned names (`dev.swaypplet.Power1`) and readers that tolerate missing keys.
- **One more thing to restart.** `nx-converge` already stages instead of switching when swaypplet changes under a locked session. A daemon adds a unit to that dance.

## Where to start, if ever

1. **In-process first.** Give `Observed` a key registry and a `Send` snapshot handle; move the settings cell to per-section change events. No new process, no protocol.
2. **One cross-process slice: power.** The idle process already runs a session-bus server for presence. Have the power service publish `OnAc` and battery state there from UPower, and have `idle::on_ac` read that, falling back to sysfs when the bus is down. One source, pushed, with the sysfs read kept as the failsafe.
3. **Inhibitors next.** Follow the units' `ActiveState` through systemd's own `PropertiesChanged` instead of `systemctl` subprocesses. That is subscribing to an existing store, not building one.
4. **Nix tables last, and differently.** The cross-repo tables are build-time data, not live state. Keep the generated `/etc/swaypplet/*.json` pattern for them; a runtime bus would not help.

Stop after step 2 if the in-process registry and the power slice do not remove code. If they only move it, the ad-hoc cells were already the right size.
