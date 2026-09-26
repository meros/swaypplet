---
name: Dual Kawase / dual filtering blur
slug: dual-kawase-blur
domain: motion
kind: pattern
platform: cross
vendor: Masaki Kawase; Marius Bjørge (ARM)
years: 2003–present
status: current
tags: [blur, kawase, dual-filter, bandwidth, mobile-gpu, gaussian]
relevance: high
---

## What it is
Kawase's 2003 GDC bloom technique blurs by repeated passes of a 4-tap filter at growing offsets, sampling between texels so bilinear filtering does extra work. Marius Bjørge's SIGGRAPH 2015 talk "Bandwidth-Efficient Rendering" (ARM) combined it with downsampling: blur while going down a half-resolution chain, then blur again on the way up ("dual filter").

## What was new
Bjørge measured it against Gaussian and plain Kawase on a mobile GPU and showed dual filtering needs a fraction of the memory bandwidth for the same visual radius, because most passes run at 1/4, 1/16, 1/64 of the pixels. Bandwidth, not ALU, is what costs power on tiled mobile and integrated GPUs.

## What went wrong / limits
It approximates a Gaussian; strength steps are discrete (per iteration). Animated strength changes can pop between iteration counts unless offsets are interpolated.

## Lessons for swaypplet
- Take: judge glass cost by bandwidth (full-resolution texture reads and writes per frame) as much as by fetch count; an iGPU laptop is a bandwidth machine.
- Avoid: animating blur radius across an iteration boundary; animate the offset or cross-fade between two fixed levels, which swaypplet's two-level frost already allows.

## Sources
- https://community.arm.com/cfs-file/__key/communityserver-blogs-components-weblogfiles/00-00-00-20-66/siggraph2015_2D00_mmg_2D00_marius_2D00_notes.pdf — dual filtering, bandwidth measurements [memory]
- https://github.com/alex47/Dual-Kawase-Blur — reference implementation by the KWin author [memory]
