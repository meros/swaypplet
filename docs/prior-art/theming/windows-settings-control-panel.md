---
name: Windows Settings vs Control Panel
slug: windows-settings-control-panel
domain: theming
kind: product
platform: Windows
vendor: Microsoft
years: 2012–present
status: current
tags: [settings-app, migration, legacy, split-brain]
relevance: medium
---

## What it is
Windows 8 introduced "PC Settings" beside the Control Panel; Windows 10 made Settings the primary app and began migrating Control Panel applets into it. As of late 2025 the migration is still incomplete: printers and drivers, advanced sound, BitLocker, some File Explorer options and recovery remain in Control Panel.

## What was new
- A touch-friendly, searchable settings app with deep links (`ms-settings:` URIs) that other apps and docs can open directly.
- Windows 11 added breadcrumbs and a home page of recommended settings.

## What went wrong / limits
- Two apps for a decade: users do not know which holds a setting; Settings often links out to an old applet with a different look.
- Microsoft cites hardware/driver diversity and enterprise dependencies for the slow move.

## Lessons for swaypplet
- Avoid a split between the pane and hand-edited files for the same setting. swaypplet's single store with a CLI (`swaypplet settings`) and Nix layer avoids it; keep sections out of other config files.
- Take deep links: a `:set look.accent` or `swaypplet settings open look` that opens the pane at a row.

## Sources
- https://www.ghacks.net/2025/12/02/windows-11-ten-years-after-it-started-microsoft-is-still-moving-control-panel-options-to-settings/ — status Dec 2025 [verified via search summary]
- https://www.makeuseof.com/windows-11-retired-control-panel-forgot-move-these-5-essential-features/ — remaining applets [verified via search summary]
- `ms-settings:` URI scheme [memory]
