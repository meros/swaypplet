---
name: Material Shell (to Veshell)
slug: material-shell
domain: wayland
kind: product
platform: GNOME
vendor: PapyElGringo and contributors
years: 2019–2023 (extension); Veshell ongoing
status: discontinued
tags: [gnome-extension, tiling, workspaces-as-apps, flutter, smithay]
relevance: low
---

## What it is
A GNOME Shell extension that replaced GNOME's UI with a left-edge panel of workspaces, each shown as a tab strip of its windows, with automatic tiling layouts per workspace. Its author ended it and started Veshell, a standalone compositor on Smithay with a Flutter UI, explaining why in a letter to users.

## What was new
- Workspaces as named, visible, persistent groups with their own layout, navigated like tabs; everything by keyboard with predictable positions.

## What went wrong / limits
- Every GNOME release broke it; the author concluded an extension cannot deliver the workflow reliably and left GNOME entirely (the letter).
- Last release around GNOME 43/44; users were stranded.

## Lessons for swaypplet
- Another case against building on a host's internals. swaypplet's position (own compositor patch, own shell) is the lesson applied.
- The "workspace as a named, tabbed group" is close to swaypplet's task workspaces; the tab strip is worth studying for the bar's workspace map.

## Sources
- https://github.com/material-shell/material-shell/blob/main/documentation/letter_for_material_shell_users.md — the letter [verified via search summary]
- https://github.com/material-shell/material-shell — transition to Veshell [verified via search summary]
