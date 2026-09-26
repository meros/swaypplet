---
name: macOS System Settings (Ventura redesign)
slug: macos-system-settings
domain: theming
kind: product
platform: macOS
vendor: Apple
years: 2022–present
status: current
tags: [settings-app, sidebar, ios-parity, criticism, search]
relevance: high
---

## What it is
macOS Ventura (2022) replaced System Preferences' icon grid with System Settings: an iOS-style sidebar of ~30 categories and scrolling grouped lists built in SwiftUI.

## What was new
- One scrolling list per pane with grouped rows, matching iPhone/iPad, so settings could share structure across platforms.
- Search in the sidebar highlights matching rows.

## What went wrong / limits
- Heavily criticised: long lists with no hierarchy, settings moved without logic (Lock Screen vs Screen Saver), fixed narrow window, slow pane switching, inconsistent controls and truncated labels during betas.
- The old grid let experts find panes by spatial memory; the sidebar broke that for all users at once.
- Settings deep inside "Info (i)" buttons are hard to discover.

## Lessons for swaypplet
- Avoid long undifferentiated lists; swaypplet's six tabs with sections is the right scale. Keep each tab short enough to fit without scrolling where possible.
- Avoid moving settings between tabs once people have learned them; if you do, make `:set` search find the old names.

## Sources
- https://www.macworld.com/article/836295/macos-ventura-system-settings-preferences-problems.html — criticism [verified via search summary]
- https://lapcatsoftware.com/articles/SystemSettings.html — detailed critique [verified via search summary]
- https://en.wikipedia.org/wiki/MacOS_Ventura — redesign [verified via search summary]
