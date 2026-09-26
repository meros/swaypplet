---
name: Focus assist automatic rules
slug: windows-focus-auto-dnd
domain: shells
kind: pattern
platform: Windows
vendor: Microsoft
years: 2018–present
status: current
tags: [do-not-disturb, context, presentation, fullscreen, summary]
relevance: high
---

## What it is
Windows 10's Focus assist (2018), now "Do not disturb" in Windows 11, can switch on by itself: during set hours, when duplicating the display (presenting), when playing a game, when an app is full screen, and for the first hour after a feature update. Windows 10 showed a summary of what was missed when Focus assist turned off.

## What was new
Context triggers instead of only a manual toggle, with presentation (display duplication) detected as a context. The "summary of what you missed" toast at the end closes the loop.

## What went wrong / limits
- Rules are off or buried by default; many users never find them.
- Duplicate-display is a proxy for presenting; screen sharing in a video call is not detected.
- The Windows 11 version dropped the end-of-session summary for a plain badge.

## Lessons for swaypplet
- Take: ROADMAP item 3 matches this closely; add the triggers Windows lacks (screen capture active via the privacy indicator), keep the end summary Windows dropped.
- Take: rules on by default for presenting and fullscreen, since the cost of a missed toast is low and the cost of one shown to an audience is high.

## Sources
- https://www.elevenforum.com/t/manage-focus-assist-automatic-rules-in-windows-11.1363/ — rule list [verified]
- https://support.microsoft.com/en-us/windows/focus-stay-on-task-without-distractions-in-windows-cbcc9ddb-8164-43fa-8919-b9a2af072382 — Focus and DND [verified]
- https://winsides.com/turn-on-do-not-disturb-automatically-windows-11/ — duplicate display, game, full screen, post-update rules [verified]
