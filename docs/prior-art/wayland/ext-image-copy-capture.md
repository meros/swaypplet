---
name: ext-image-copy-capture-v1 and ext-image-capture-source-v1
slug: ext-image-copy-capture
domain: wayland
kind: protocol
platform: Wayland
vendor: Andri Yngvason and others; wayland-protocols
years: 2024–present
status: current
tags: [protocol, screencopy, toplevel-capture, damage, dmabuf]
relevance: high
---

## What it is
The standard successor to `wlr-screencopy`. A capture source (an output, or a toplevel from `ext-foreign-toplevel-list`) is opened as a session; the client allocates shm or dmabuf buffers to the advertised constraints and receives frames with damage regions. A separate cursor session captures the pointer. sway 1.11 shipped output capture; swaypplet uses both output and toplevel sources for Super+Tab and pins.

## What was new
- Per-toplevel capture as a standard, so live window thumbnails no longer need compositor plugins (compare hyprexpo).
- Damage reporting per frame, so a client copies only changed regions.
- Buffer constraints negotiated up front, including dmabuf formats and modifiers.

## What went wrong / limits
- Toplevel capture arrived later than output capture in wlroots/sway.
- Using shm for full-size frames costs a CPU copy each frame; dmabuf is needed for anything full-screen (roadmap item 4 names this).

## Lessons for swaypplet
- Use damage to skip re-uploading unchanged tiles.
- Move large captures to dmabuf (`GdkDmabufTextureBuilder`) before building the overview.

## Sources
- https://github.com/swaywm/sway/pull/7976 — sway implementation [verified via search summary]
- https://wayland.app/protocols/ext-image-copy-capture-v1 — protocol [memory]
