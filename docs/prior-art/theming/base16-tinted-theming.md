---
name: base16 / tinted-theming
slug: base16-tinted-theming
domain: theming
kind: protocol
platform: cross
vendor: Chris Kempson; tinted-theming community
years: 2012–present
status: current
tags: [scheme, templates, 16-colours, base24, builder]
relevance: medium
---

## What it is
A scheme format of 16 named colours (base00–base07 a dark-to-light neutral ramp, base08–base0F accents) with a styling guide assigning roles (base08 variables/red, base0B strings/green, base0D functions/blue), and a builder that renders mustache templates for hundreds of apps. Maintained since 2022 by tinted-theming, with base24 adding 8 colours for full ANSI support, and `tinty` as a scheme manager.

## What was new
- Decoupled schemes from templates: N schemes × M apps from N + M files.
- A neutral ramp by position (00→07) is a primitive scale in all but name.

## What went wrong / limits
- Roles are syntax-highlighting roles; UI roles (accent, surface, status) are inferred differently per template.
- The original project stalled for years before the community fork; spec versions and builders diverged.
- No contrast rules; light schemes invert the ramp inconsistently.

## Lessons for swaypplet
- A base16 export is the widest-reach format for other apps (Stylix, tinty, editors). Map neutral steps 1–12 onto base00–07 and the categorical slots onto base08–0F.

## Sources
- https://github.com/tinted-theming/home — project, style guide [verified via search summary]
- https://github.com/tinted-theming/base24/blob/main/README.md — base24 [verified via search summary]
