---
name: Cmd+K palettes in web apps (Superhuman, Linear, Slack, Notion)
slug: cmd-k-web-apps
domain: launchers
kind: pattern
platform: web
vendor: Superhuman, Linear, Slack, Notion, and others
years: 2017–present
status: current
tags: [cmd-k, command palette, shortcut training, context actions]
relevance: medium
---

## What it is
A convention that spread from Superhuman (2017) and Linear (2019) to most productivity web apps: Cmd+K opens a palette of every action, filtered by context (the selected email, issue or page). Slack's Cmd+K is a channel/DM switcher; Notion's is page search.

## What was new
- **Context-scoped actions**: the palette lists what applies to the current selection first.
- **Shortcut training**: Superhuman shows the shortcut beside each command and nudges users to use it; onboarding measures shortcut adoption.
- Libraries (cmdk, kbar, ninja-keys) made it a commodity.

## What went wrong / limits
- Every app has its own palette, own shortcut, own ranking; the OS launcher cannot see into them.

## Lessons for swaypplet
- Take context first: the launcher can offer actions for the focused window (move to workspace, float, fullscreen, kill) above search results.

## Sources
- https://blog.superhuman.com/how-to-build-a-remarkable-command-palette/ — design notes [memory]
- https://github.com/pacocoursey/cmdk — library [memory]
