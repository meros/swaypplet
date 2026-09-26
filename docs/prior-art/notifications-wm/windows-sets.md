---
name: Windows Sets (tabbed windows)
slug: windows-sets
domain: notifications-wm
kind: feature
platform: Windows
vendor: Microsoft
years: 2017–2019
status: failed
tags: [tabs, window-groups, cancelled, cross-app]
relevance: low
---

## What it is
A Windows 10 Insider feature (2017–2018) that let any app's windows be tabs in one frame, mixing Edge pages, Word documents and File Explorer, with tab groups resumable later. Pulled from Insider builds in June 2018; confirmed dead in April 2019.

## What was new
Tabs as a system-level container for mixed apps, a task-grouping unit like Rooms but inside one window.

## What went wrong / limits
It depended on apps redrawing their title bars (UWP and Edge's EdgeHTML engine); Edge's switch to Chromium and Win32 app compatibility pulled the ground away. File Explorer tabs arrived alone in Windows 11 in 2022.

## Lessons for swaypplet
- Note: sway's tabbed and stacked containers already are Sets, for every app, because the compositor owns the decorations. No shell work needed; the lesson is that WM-level grouping beats app-cooperative grouping.

## Sources
- https://www.bleepingcomputer.com/news/microsoft/microsoft-discontinues-windows-10-sets-tabbed-window-feature/ — discontinuation. [verified via search summary]
- https://www.tomshardware.com/news/windows-10-insider-build-17704-sets-removed,37379.html — June 2018 removal. [verified via search summary]
- https://www.howtogeek.com/411932/windows-10s-sets-app-tabs-are-no-more/ — Edge Chromium change. [verified via search summary]
