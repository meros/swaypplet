---
name: Arc command bar
slug: arc-command-bar
domain: launchers
kind: feature
platform: cross
vendor: The Browser Company
years: 2022–2025
status: discontinued
tags: [browser, command bar, tabs, history, commands, sunset]
relevance: low
---

## What it is
Arc's Cmd+T opened one bar that searched open tabs, history, bookmarks, browser commands and the web, instead of just opening a blank tab.

## What was new
- **New tab = command bar**: the most frequent browser action became a search over what already exists, so duplicates dropped ("switch to tab" ranked above "open").
- Browser settings and actions as rows.

## What went wrong / limits
- The Browser Company stopped feature work on Arc in 2025 to build Dia, an AI-first browser, and was acquired by Atlassian that year; Arc went into maintenance.

## Lessons for swaypplet
- Take "switch to existing before launching new": when an app row has an open window, offer focus as the first action and launch-new as the second; swaypplet's Tab already lists windows, the default action could prefer them.

## Sources
- https://arc.net/ — command bar [memory]
- https://browsercompany.substack.com/ — Arc maintenance, Dia (2025) [memory]
