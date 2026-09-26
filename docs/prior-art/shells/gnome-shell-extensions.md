---
name: GNOME Shell extensions
slug: gnome-shell-extensions
domain: shells
kind: pattern
platform: GNOME
vendor: GNOME Project and community
years: 2011–present
status: current
tags: [extensibility, monkey-patching, breakage, plugins]
relevance: medium
---

## What it is
Extensions are JavaScript modules loaded into the GNOME Shell process that may modify any part of it at runtime. extensions.gnome.org distributes them with review. They restore features the core removed (dock, tray icons, app menu) and add new ones.

## What was new
Total access: any shell behaviour can be changed without forking, which let the core stay minimal while users rebuild what they want.

## What went wrong / limits
- Extensions work by monkey-patching internal classes, so a shell release breaks many even when no public API changed. GNOME 45 (2023) moved to ES modules and broke every extension at once.
- A faulty extension crashes or slows the whole shell (same process; on Wayland the compositor itself).
- The core team's minimalism is sustained by the extensions, but users pay at every upgrade.

## Lessons for swaypplet
- Avoid: an in-process plugin API with no stable surface. If swaypplet ever exposes extension points, make them data (JSON from Nix, as `/etc/swaypplet/*.json` already are) or out-of-process (D-Bus).
- Note: the cross-repo guard in the nixos repo is the small-scale version of this problem; the generated-file pattern is the fix.

## Sources
- https://blogs.gnome.org/shell-dev/2023/09/02/extensions-in-gnome-45/ — ESM migration [verified]
- https://gjs.guide/extensions/overview/updates-and-breakage.html — monkey-patching and breakage [verified]
