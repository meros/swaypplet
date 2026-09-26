---
name: Windows Timeline
slug: windows-timeline
domain: notifications-wm
kind: feature
platform: Windows
vendor: Microsoft
years: 2018–2021
status: discontinued
tags: [activity-history, resume, cloud-sync, privacy, task-view]
relevance: low
---

## What it is
Added to Task View in the Windows 10 April 2018 update: a scrollable history of past "activities" (documents, web pages) across 30 days and devices, to resume work. Cloud sync for consumer accounts stopped in 2021, the UI is absent from Windows 11, and enterprise sync ended in January 2024.

## What was new
Resuming by time rather than by file location; cross-device continuity.

## What went wrong / limits
Depended on apps publishing User Activities, which few did beyond Microsoft's own; the history was patchy. Sending activity to the cloud raised privacy concerns. Recall is the same idea done by screenshots, with worse privacy.

## Lessons for swaypplet
- Avoid: activity history features that need app cooperation or collect a log of the owner's work.

## Sources
- https://support.microsoft.com/en-us/windows/get-help-with-timeline-febc28db-034c-d2b0-3bbe-79aa0c501039 — Timeline retired in Windows 11. [verified via search summary]
- https://www.atera.com/blog/activity-history-in-windows-full-user-guide/ — sync end dates. [verified via search summary]
- April 2018 launch and app adoption. [memory]
