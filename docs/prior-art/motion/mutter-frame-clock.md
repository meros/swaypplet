---
name: Mutter frame clock and frame timings
slug: mutter-frame-clock
domain: motion
kind: feature
platform: GNOME
vendor: GNOME
years: 2020–present
status: current
tags: [frame-clock, per-output, scheduling, max-render-time, sysprof, latency]
relevance: high
---

## What it is
GNOME 3.38 (2020) replaced Clutter's global master clock with a `ClutterFrameClock` per output, so each monitor paints at its own refresh rate. GNOME 40+ schedules the paint as late as possible before vblank based on measured render time, and Sysprof shows per-frame timings for Mutter and GTK.

## What was new
- Mixed refresh rate multi-monitor finally worked: a 144 Hz and a 60 Hz monitor each got their own clock.
- Paint start is chosen from a running estimate of how long the last frames took, trading a little headroom for lower input latency. sway has a manual version, `max_render_time`.

## What went wrong / limits
The estimate is fragile on GPUs that downclock under light load: an idle-ish desktop renders slowly *because* it is idle, misses the late deadline, and stutters. That problem led to the triple-buffering work.

## Lessons for swaypplet
- Take: sway's `max_render_time` and swaypplet's frame work together set the latency; a gate that only checks swaypplet's work cannot see a compositor render deadline miss. Record both on the same frame.
- Take: Sysprof-style marks (`GDK_DEBUG=frames` is already used) are the right primitive; export them as spans so a trace viewer shows animation, snapshot and present on one timeline.

## Sources
- https://gitlab.gnome.org/GNOME/mutter/-/merge_requests/1285 — per-output frame clock rework [memory]
- https://blogs.gnome.org/shell-dev/ — frame scheduling posts [memory]
