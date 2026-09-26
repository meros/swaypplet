---
name: KDE Activities
slug: kde-activities
domain: shells
kind: feature
platform: KDE
vendor: KDE
years: 2010–present
status: niche
tags: [contexts, workspaces, failed-idea, desktop-layout]
relevance: low
---

## What it is
Activities (Plasma 4.5+) are named contexts, each with its own desktop widgets, wallpaper, recent files and windows, and optionally its own power settings. Windows can belong to one or several activities. They coexist with virtual desktops.

## What was new
A context bigger than a workspace: switching activity changes the desktop, recent documents and available windows together, and an activity can be stopped and resumed.

## What went wrong / limits
- Two overlapping concepts (virtual desktops and activities) with an unclear difference; most users never create a second activity.
- Apps had to integrate with the activity manager for recent documents to be per-activity; few did.
- Developers repeatedly discussed simplifying or removing them; they remain but are de-emphasized in Plasma 6.

## Lessons for swaypplet
- Avoid: a second axis of grouping on top of workspaces. swaypplet's task workspaces (numbered groups per task) already are its activities; do not add another layer.
- Note: per-context behaviour (quiet mode, power) is better keyed to detectable context (ROADMAP item 3) than to a manual mode the user must switch.

## Sources
- https://userbase.kde.org/Plasma/Activities — feature description [memory]
- https://en.wikipedia.org/wiki/KDE_Plasma_4 — introduction [memory]
