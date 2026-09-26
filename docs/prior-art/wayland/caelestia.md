---
name: Caelestia shell
slug: caelestia
domain: wayland
kind: project
platform: Wayland
vendor: caelestia-dots (Soramane)
years: 2024–present
status: current
tags: [quickshell, morphing, motion, hyprland, dashboard]
relevance: high
---

## What it is
"A fluid, morphing shell": a Quickshell config for Hyprland in which the bar, launcher, dashboard (media, performance, weather) and notifications are drawn as one continuous shape that grows and morphs out of the screen edge rather than opening separate windows. It ships a CLI (`caelestia shell -d`).

## What was new
- Surfaces as extensions of one border: a panel grows out of the bar's edge with continuous corners, so opening a panel reads as the shell changing shape, not a window appearing.
- Motion treated as the product's identity, with interruptible QtQuick animations.

## What went wrong / limits
- Hyprland-first; niri and sway support lag.
- Heavy motion everywhere conflicts with a still-at-rest principle; every open and close animates.

## Lessons for swaypplet
- The morph is exactly swaypplet's rejected "bar-to-panel morph"; Caelestia shows it works visually, but the owner's rejection stands. Note it as evidence, not a reopen.
- Take: shared-edge geometry (panel corners continuing the bar's) as a cheaper cue than a full morph.

## Sources
- https://github.com/caelestia-dots/shell — description and components [verified via search summary]
