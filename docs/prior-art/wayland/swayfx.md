---
name: SwayFX (and scenefx)
slug: swayfx
domain: wayland
kind: product
platform: Wayland
vendor: WillPower3309 and the wlrfx organisation
years: 2022–present
status: current
tags: [compositor, blur, shadows, rounded-corners, scenefx, fork]
relevance: high
---

## What it is
A fork of sway that swaps wlroots' plain renderer for scenefx, a drop-in replacement for the wlroots scene graph with GLES effects: dual-kawase background blur, drop shadows, anti-aliased rounded corners, dim of inactive windows. swaypplet runs on a patched swayfx and its liquid-glass shader lives there.

## What was new
- `layer_effects "<namespace>" blur enable; shadows enable; corner_radius N` applies effects to layer-shell surfaces by namespace, so a bar gets blur without any client protocol.
- scenefx is a separate library, so other wlroots compositors can reuse the effects instead of forking the renderer.
- Keeps full sway config compatibility; the fork is additive.

## What went wrong / limits
- Always a release behind sway, because every wlroots bump must be ported through scenefx.
- Effects are compositor configuration keyed by namespace, not client-requested; a client cannot ask for blur on a region, and blur follows the whole surface (swaypplet's "binary frost" issue with alpha < 1).
- A fork of a fork (swaypplet's patched swayfx) multiplies the rebase burden.

## Lessons for swaypplet
- Keep the namespace-to-effect table generated from Nix (already done via `glass-config.nix`); it is the stable seam.
- Watch `ext-background-effect-v1`: if scenefx implements it, the shell could request blur per region and drop part of the local patch.
- Budget a rebase per sway release; the patch set is a standing cost.

## Sources
- https://github.com/wlrfx/swayfx — feature list, `layer_effects` syntax [verified via search summary]
- https://creitingameplays.github.io/swayfx-enhanced/ — a separate swayfx fork with liquid glass [verified: exists]
