---
name: GTK4 GSK renderers (ngl and vulkan)
slug: gtk4-gsk-unified-renderers
domain: motion
kind: feature
platform: GNOME
vendor: GNOME (GTK)
years: 2024–present
status: current
tags: [gtk4, gsk, render-nodes, vulkan, ngl, ubershader, fractional-scaling]
relevance: high
---

## What it is
GTK4 draws via a render-node tree (GSK). In January 2024 GTK announced two new renderers, `ngl` (GL) and `vulkan`, built from one shared source modelled on Vulkan APIs. `ngl` became the default in 4.14; `vulkan` became the Wayland default in 4.16. `GSK_RENDERER=gl|ngl|vulkan|cairo` overrides the choice.

## What was new
Correct antialiasing, fractional scaling done in the renderer, and unlimited gradient stops, via ubershaders that interpret a node buffer rather than a shader per node type.

## What went wrong / limits
- The GTK blog said plainly: "the new renderers are not faster (yet)"; new ngl was slower than old gl for undetermined reasons.
- Driver issues on the new usage patterns; Ubuntu considered reverting to ngl for 24.10 over regressions.
- Fractional positions are no longer rounded, so blurry text appears if a widget lands on half a pixel; animated translations can soften text mid-motion.

## Lessons for swaypplet
- Take: frame-bench should record `GSK_RENDERER` and run the gate on the renderer the session actually uses; a GTK update can switch it.
- Take: snap animated offsets to device pixels at rest (and preferably while text is moving) to avoid soft text; check at scale 1.25.

## Sources
- https://blog.gtk.org/2024/01/28/new-renderers-for-gtk/ — unified renderers, "not faster (yet)", fractional positions, GSK_RENDERER [verified]
- https://www.phoronix.com/news/GTK-4.16-Released — Vulkan default on Wayland in 4.16 [verified via search summary]
- https://bugs.launchpad.net/bugs/2082017 — Ubuntu considering reverting to ngl [verified via search summary]
