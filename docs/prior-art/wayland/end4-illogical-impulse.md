---
name: end-4 dots-hyprland (illogical-impulse)
slug: end4-illogical-impulse
domain: wayland
kind: project
platform: Wayland
vendor: end-4
years: 2023–present
status: current
tags: [dotfiles, hyprland, quickshell, material-you, sidebars, ai]
relevance: medium
---

## What it is
The most-starred Hyprland dotfiles: a complete shell (bar, left and right sidebars, overview, OSD, notifications, lock) called illogical-impulse. It started on AGS; the current version is Quickshell, and the AGS version survives on an `ii-ags` branch.

## What was new
- Material You colour generation from the wallpaper applied across shell, GTK, Qt and terminal.
- Sidebars as a place for heavier tools (translator, AI chat, to-do) separate from the bar.
- Workspace overview drawn by the shell with live window previews.

## What went wrong / limits
- The stack migration (AGS to Quickshell) forced a reinstall; installers are distro scripts, and "instant lock at login" style bugs track Hyprland changes.
- Dotfiles as product: users inherit hundreds of choices they did not make and cannot update cleanly.

## Lessons for swaypplet
- Take the wallpaper-derived palette applied everywhere, which swaypplet's tint already does; make sure GTK and Qt apps follow.
- Avoid a shell that is also an installer; Nix already solves distribution.

## Sources
- https://github.com/end-4/dots-hyprland — project, Quickshell migration [verified via search summary]
- https://github.com/end-4/dots-hyprland/issues/2808 — instant locking bug [verified via search summary]
