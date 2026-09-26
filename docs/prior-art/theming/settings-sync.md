---
name: Settings sync (Windows, macOS, VS Code, browsers)
slug: settings-sync
domain: theming
kind: pattern
platform: cross
vendor: Microsoft, Apple, Google, others
years: 2012–present
status: current
tags: [sync, cloud, portability, dotfiles, conflicts]
relevance: low
---

## What it is
Cloud sync of preferences across devices: Windows "Remember my preferences" (theme, passwords, language) via Microsoft account since Windows 8; macOS/iOS iCloud for some settings; Chrome/Firefox sync; VS Code Settings Sync (2020) with per-category toggles and a conflict merge view.

## What was new
- VS Code: per-category choice (settings, keybindings, extensions, UI state), machine-specific settings excluded by schema (`"scope": "machine"`), and a diff/merge UI on conflict.
- Browsers: sync as account feature, making a new device feel like yours in minutes.

## What went wrong / limits
- OS-level sync is patchy and opaque: users cannot see what synced; Windows sync was trimmed over time.
- Syncing machine-specific values (display scale, paths) breaks the other machine unless scoped.

## Lessons for swaypplet
- Nix already is the sync mechanism (the system layer); the user file is per machine. Borrow VS Code's scope idea: mark sections as machine-local (displays, wallpaper path) vs portable (look, bar), and have "Copy as Nix" export only portable sections into the shared config.

## Sources
- https://code.visualstudio.com/docs/editor/settings-sync — categories, machine scope, merge [memory]
- Windows sync settings history [memory]
