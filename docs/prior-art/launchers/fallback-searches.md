---
name: Fallback searches
slug: fallback-searches
domain: launchers
kind: pattern
platform: cross
vendor: Alfred, Albert, PowerToys Command Palette, Chrome keyword search
years: 2010–present
status: current
tags: [no results, web search, explicit network, empty state]
relevance: high
---

## What it is
When nothing (or little) matches, the launcher shows a short, user-ordered list of actions that take the typed text as input: search the web, search a wiki, run in terminal, look up in a dictionary.

## What was new
- Turns "no results" into a useful state without mixing web results into local ones.
- The network is only touched when the user picks the row, so the query leaves the machine by choice.
- Alfred lets fallbacks be any workflow; Command Palette's `??` prefix is the same idea behind a prefix.

## What went wrong / limits
- Nothing notable; the main failure is the opposite design (web results by default, see [windows-start-search](windows-start-search.md)).

## Lessons for swaypplet
- Take it: when the result list would be empty or below three rows, append "Search the web for 'q'", "Run 'q'" (the `>` row), and "Search files for 'q'". S.

## Sources
- https://www.alfredapp.com/help/features/default-results/fallback-searches/ — Alfred fallbacks [memory]
- https://learn.microsoft.com/en-us/windows/powertoys/command-palette/overview — `??` web search [verified]
