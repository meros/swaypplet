---
name: Quickshell
slug: quickshell
domain: wayland
kind: project
platform: Wayland
vendor: outfoxxed and contributors
years: 2024–present
status: current
tags: [qml, qtquick, hot-reload, toolkit, shells]
relevance: high
---

## What it is
A toolkit for building bars, panels, lock screens and widgets in QML on Qt6/QtQuick. It wraps layer-shell, session lock, screencopy, toplevel management, PipeWire, MPRIS, tray, PAM, Bluetooth, Hyprland and i3/sway IPC as QML types. From 2025 it is the base of most prominent riced shells (end-4, Caelestia, DankMaterialShell, Noctalia).

## What was new
- Hot reload with a generation-based engine: save the file and the shell reloads without a process restart, keeping iteration under a second.
- QtQuick's scene graph gives GPU-rendered, interruptible animations and shaders cheaply, which is why its shells look far smoother than GTK3 ones.
- `BackgroundEffect` type (v0.3) exposes `ext-background-effect-v1` blur to QML.
- `PanelWindow`, `WlSessionLock`, `ScreencopyView` as declarative types.

## What went wrong / limits
- Shells written in it are large QML trees with ad-hoc JS; quality varies and startup memory is high (hundreds of MB for big shells) [memory].
- Forks of the toolkit itself (noctalia-qs) show the fragmentation starting again.

## Lessons for swaypplet
- Take hot reload for CSS and design tokens at least: live token edits would speed zoo work.
- Take `ScreencopyView` as a concept: a widget that is a live capture of a toplevel, which is what pins and Super+Tab tiles are.

## Sources
- https://quickshell.org/about/ — scope and hot reload [verified via search summary]
- https://github.com/quickshell-mirror/quickshell — integrations [verified via search summary]
- https://quickshell.org/docs/v0.3.1/types/Quickshell.Wayland/BackgroundEffect/ — blur type [verified via search summary]
