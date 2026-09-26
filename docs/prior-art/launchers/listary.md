---
name: Listary
slug: listary
domain: launchers
kind: product
platform: Windows
vendor: Bopsoft
years: 2010–present
status: current
tags: [type-anywhere, file dialogs, quick switch, context]
relevance: medium
---

## What it is
A file search and launcher for Windows that also attaches to Explorer and to every Open/Save dialog: start typing in a file window and a search box appears there.

## What was new
- **Search where you already are**: type-to-find inside file managers and dialogs, not in a separate window.
- **Quick Switch**: in an Open/Save dialog, one key jumps the dialog to the folder currently open in Explorer (or a recent one).

## What went wrong / limits
- Relies on hooking other apps' windows, which breaks with OS updates and is impossible on Wayland without the toolkit's cooperation.

## Lessons for swaypplet
- Adapt Quick Switch: the xdg-desktop-portal file chooser could be offered "the folder of the focused file manager window"; a launcher row can do the same ("Open recent folder in…").

## Sources
- https://www.listary.com/ — features, Quick Switch [memory]
