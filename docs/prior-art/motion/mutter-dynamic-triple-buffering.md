---
name: Mutter dynamic triple buffering
slug: mutter-dynamic-triple-buffering
domain: motion
kind: feature
platform: GNOME
vendor: Canonical / GNOME
years: 2020–2025
status: current
tags: [triple-buffering, frame-pacing, gpu-clock, latency, upstreaming]
relevance: high
---

## What it is
A Mutter change by Daniel van Vugt (Canonical, MR !1441) that switches to triple buffering only when the previous frame ran late, and stays double buffered otherwise. Ubuntu shipped it as a downstream patch from 22.04 (2022); it merged upstream for GNOME 48 (2025).

## What was new
- Latency cost only when needed: if the previous frame is late, the next one is dispatched on time instead of also starting late; when frames keep up, behaviour is plain double buffering.
- It exposed a counter-intuitive cause of jank: with strict double buffering on Intel iGPUs and the Raspberry Pi, the GPU idles between frames, the driver lowers its clock, and the next frame is then slower. Keeping the GPU busier restores clocks; a 4K overview animation went from about 30 back to 60 fps.

## What went wrong / limits
Four-plus years from prototype to merge. Review centred on latency and on Mutter's single-threaded event loop, and the patch was rewritten several times. The debrief notes apps (GTK, Firefox) still stutter in inertial scrolling unless the machine is in performance mode, because the toolkits do not do the same.

## Lessons for swaypplet
- Take: a light animation on an iGPU can jank *because* it is light. frame-bench on a headless backend cannot see this; a real-hardware run under `powersave` is needed.
- Avoid: long-lived downstream patches in a compositor. swaypplet carries swayfx patches; the cost of rebasing grows with every upstream frame-scheduling change.

## Sources
- https://gitlab.gnome.org/GNOME/mutter/-/merge_requests/1441 — mechanism, overview 30 fps problem [verified]
- https://discourse.ubuntu.com/t/triple-buffering-a-debrief/56314 — timeline, latency objections, remaining app issues [verified]
- https://www.phoronix.com/news/GNOME-48-Triple-Buffering — merged for GNOME 48 [verified via search summary]
