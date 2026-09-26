---
name: macOS Dock
slug: macos-dock
domain: shells
kind: feature
platform: macOS
vendor: Apple (from NeXT)
years: 2001–present
status: current
tags: [dock, launcher, running-indicator, magnification, minimize]
relevance: low
---

## What it is
A strip of app icons on the bottom or side of the screen that combines pinned launchers and running apps, with a divider before documents, folders ("stacks") and the Trash. Running apps show a small dot. Minimized windows go into the Dock with the Genie or Scale effect.

## What was new
Merged launcher and task list into one row keyed by app, not window. Magnification on hover (optional) and bounce-to-request-attention. Descended from the NeXTSTEP dock (see nextstep-dock).

## What went wrong / limits
- App-centric: several windows of one app sit behind one icon, so window-level switching goes through Mission Control or the app's Window menu.
- The attention bounce loops until acknowledged, a known annoyance; it is one of few continuous motions in the shell.
- Auto-hide has a reveal delay that Apple never exposed in settings (only via `defaults`).

## Lessons for swaypplet
- Avoid: a looping attention animation. BAR_VISION P2 (onset-only motion) already rejects the bounce.
- Adapt: the running-indicator dot is a pre-attentive mark with a single meaning; swaypplet's board bays use the same economy.
- Not needed: a tiling WM with numbered workspaces makes an app dock redundant.

## Sources
- https://en.wikipedia.org/wiki/Dock_(macOS) — history, features, NeXT lineage [memory]
- https://support.apple.com/guide/mac-help/use-the-dock-on-mac-mh35859/mac — dock behaviour and settings [memory]
