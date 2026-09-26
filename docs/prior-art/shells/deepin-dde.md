---
name: Deepin Desktop Environment (DDE)
slug: deepin-dde
domain: shells
kind: product
platform: other
vendor: Deepin Technology (UnionTech)
years: 2011–present
status: current
tags: [dock-modes, control-center, polish, qt, trust]
relevance: low
---

## What it is
The desktop of deepin (China), built on Qt, with a dock that switches between "Fashion" mode (a floating centred macOS-like dock) and "Efficient" mode (a full-width Windows-like taskbar), a slide-in Control Center sidebar, a launcher in full-screen or small mode, and heavy use of blur and animation. DDE 23 (2024) rewrote large parts and added an AI assistant.

## What was new
- One dock with two named modes toggled from its context menu, so users coming from Windows or macOS choose their mental model.
- Control Center as a right sidebar for quick settings that expands into the full settings app.

## What went wrong / limits
- Performance: blur and animations on a Qt stack were heavy on older hardware.
- Trust: early versions shipped CNZZ web analytics in the app store, removed after criticism (2018); Fedora dropped its DDE packages in 2023 over maintenance and security concerns.
- Ports to other distributions lag behind deepin itself.

## Lessons for swaypplet
- Avoid: telemetry or network calls in shell surfaces; one incident defines a desktop's reputation for years.
- Note: two named layouts of one bar is cheaper than a free-form panel editor, if a second layout is ever wanted.

## Sources
- https://en.wikipedia.org/wiki/Deepin — history, analytics controversy [memory]
- https://fedoraproject.org/wiki/Changes/Retire_Deepin — Fedora retiring DDE [memory]
