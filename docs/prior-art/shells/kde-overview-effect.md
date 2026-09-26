---
name: Plasma Overview and Desktop Grid
slug: kde-overview-effect
domain: shells
kind: feature
platform: KDE
vendor: KDE
years: 2021–present
status: current
tags: [overview, workspaces, gestures, kwin-effect, search]
relevance: high
---

## What it is
KWin's Overview effect (Plasma 5.23, 2021) shows the current virtual desktop's windows with a desktop bar and KRunner search. In Plasma 6.0 it merged with Desktop Grid into one effect with states: Meta+W overview, Meta+G grid of all desktops, Meta+Tab cycling between them, and touchpad gestures that scrub continuously between desktop, overview and grid.

## What was new
One effect with several zoom levels that the same gesture moves through, rather than separate effects that each animate in from zero. Search (KRunner) is embedded, so typing in the overview launches or finds windows.

## What went wrong / limits
- Performance with many windows on weaker GPUs; the effect is QML inside the compositor.
- Earlier Plasma had Present Windows, Desktop Grid and Overview as three overlapping effects; consolidating them took three years.

## Lessons for swaypplet
- Take: overview and workspace grid as states of one surface on one gesture axis (ROADMAP item 4); do not build a separate grid later.
- Take: embed the launcher search in the overview.
- Avoid: parallel near-duplicate surfaces (Present Windows vs Overview) that users must learn separately.

## Sources
- https://www.omgubuntu.co.uk/2024/02/kde-plasma-6-0-new-features — merge and shortcuts [verified]
- https://pointieststick.com/2023/09/29/this-week-in-kde-time-for-the-new-features/ — merged effect, gestures [verified]
