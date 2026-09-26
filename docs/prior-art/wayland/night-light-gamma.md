---
name: Night light on Wayland (wlr-gamma-control, wlsunset, gammastep, hyprsunset)
slug: night-light-gamma
domain: wayland
kind: pattern
platform: Wayland
vendor: wlroots; Kenny Levinsen (wlsunset); gammastep maintainers; hyprwm
years: 2019–present
status: current
tags: [protocol, gamma, colour-temperature, ctm, exclusive]
relevance: medium
---

## What it is
`zwlr_gamma_control_v1` lets one client set each output's gamma ramps. wlsunset and gammastep (a Redshift fork) compute a colour temperature from time or location and write ramps. Hyprland moved to a colour-transform-matrix protocol (`hyprland-ctm-control`) used by hyprsunset, which also works with HDR [memory]. swaypplet does night light in-process through gamma control since 2026-09-26.

## What was new
- wlsunset: a few hundred lines, smooth transitions over a configurable duration, no location service.
- CTM instead of gamma: a matrix applied in the compositor's colour pipeline, composable with HDR and colour management where gamma ramps are not.

## What went wrong / limits
- Gamma control is exclusive per output: a second client (a night-light daemon plus a calibration tool) gets `failed`. Ramps are lost on VT switch or DPMS on some drivers and must be reapplied.
- Gamma LUTs conflict with HDR and colour-managed outputs; wlroots 0.20's colour management makes this more visible.

## Lessons for swaypplet
- Reapply ramps on output re-enable and on `failed`, and surface "another client owns gamma" in the panel instead of failing silently.
- Watch for a colour-management-friendly path (CTM) once HDR lands in swayfx.

## Sources
- https://wayland.app/protocols/wlr-gamma-control-unstable-v1 — protocol [memory]
- https://sr.ht/~kennylevinsen/wlsunset/ — wlsunset [memory]
- https://github.com/hyprwm/hyprsunset — hyprsunset [memory]
