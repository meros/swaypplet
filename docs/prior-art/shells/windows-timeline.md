---
name: Windows Timeline
slug: windows-timeline
domain: shells
kind: feature
platform: Windows
vendor: Microsoft
years: 2018–2021
status: failed
tags: [activity-history, task-view, cross-device, privacy, failed-idea]
relevance: low
---

## What it is
Timeline (Windows 10 1803, 2018) extended Task View with a scrollable history of past activities (documents, web pages, app states) going back 30 days, synced across devices through a Microsoft account. Apps had to publish "User Activities" for their items to appear.

## What was new
Resumable activities rather than files or windows: "the document you were editing on Tuesday", reachable from the window switcher.

## What went wrong / limits
- Depended on app developers adopting the Activity API; few did beyond Edge and Office, so history was sparse.
- Cloud sync of activity raised privacy concerns; Microsoft ended sync in 2021 and Windows 11 removed Timeline.
- Mixed with Task View, it made the window switcher slower and busier.

## Lessons for swaypplet
- Avoid: features whose value depends on every app adopting a new API.
- Avoid: attaching history to the window switcher; keep Super+Tab about now.

## Sources
- https://www.techradar.com/news/microsoft-is-killing-off-one-of-the-best-windows-10-work-tools — sync ending 2021 [verified]
- https://www.digitalcitizen.life/what-is-timeline-how-use-resume-past-activities/ — feature description [verified]
