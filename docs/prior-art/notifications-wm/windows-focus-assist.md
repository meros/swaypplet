---
name: Windows Quiet Hours, Focus Assist and Do Not Disturb
slug: windows-focus-assist
domain: notifications-wm
kind: feature
platform: Windows
vendor: Microsoft
years: 2015–present
status: current
tags: [dnd, automatic-rules, fullscreen, game, duplicating-display, summary]
relevance: high
---

## What it is
Windows 10's Quiet Hours became Focus Assist in the April 2018 update: Off, Priority only (a user priority list of people and apps) and Alarms only. Automatic rules: during set times, when duplicating the display, when playing a game, when an app is fullscreen, and for the first hour after a feature update. Windows 11 22H2 renamed it Do Not Disturb and added Focus sessions (a timer tied to Clock).

## What was new
Context rules that switch quiet on and off by themselves, including a fullscreen rule and a projection rule. An option to show a summary of what was missed when Focus Assist turns off.

## What went wrong / limits
Focus Assist itself announced that it had turned on, via a notification, which How-To Geek called annoying: the stand-down message was louder than the thing it suppressed. Fullscreen detection misfires on borderless-windowed games and video players.

## Lessons for swaypplet
- Take: the rule set is almost exactly roadmap item 3; the "summary of what you missed" is direct precedent for the "7 while you were presenting" card.
- Avoid: announcing that quiet mode turned on; announce only the end, and only if something was held.
- Take: an explicit priority breakthrough list, kept short.

## Sources
- https://www.elevenforum.com/t/manage-focus-assist-automatic-rules-in-windows-11.1363/ — automatic rules list. [verified via search summary]
- https://www.tenforums.com/tutorials/102201-change-focus-assist-automatic-rules-windows-10-a.html — rules including first hour after update. [verified via search summary]
- https://www.howtogeek.com/435349/how-to-disable-windows-10s-annoying-focus-assist-notifications/ — Focus Assist's own notifications. [verified via search summary]
- Summary-of-missed option; 2018 rename; 22H2 rename. [memory]
