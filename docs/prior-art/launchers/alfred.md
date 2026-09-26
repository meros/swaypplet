---
name: Alfred
slug: alfred
domain: launchers
kind: product
platform: macOS
vendor: Running with Crayons
years: 2010–present
status: current
tags: [workflows, script filter, universal actions, file buffer, fallback searches, powerpack]
relevance: high
---

## What it is
A paid-upgrade (Powerpack) launcher for macOS. The free core launches apps, searches files and the web; the Powerpack adds workflows, clipboard history, snippets and Universal Actions.

## What was new
- **Workflows** (Alfred 2, 2013): a visual graph of triggers, inputs, actions and outputs. A *Script Filter* is any script that prints JSON items; Alfred renders them as rows and re-runs the script as you type.
- **Universal Actions** (4.5, 2021): pick an item first (file, text, URL, even selected text in another app) and then choose from 60+ actions: object first, verb second.
- **File Buffer**: Alt+Up collects several files at the top of the window; Alt+Right acts on all of them.
- **Fallback searches**: when nothing matches, the rows become "Search Google/Wikipedia/... for 'query'", configurable per user.
- Learns which result you pick for a given keyword and promotes it for that keyword.

## What went wrong / limits
- Workflows are unsandboxed scripts with the user's full rights; the Gallery is curated, but side-loaded `.alfredworkflow` files are trusted blindly.
- The graph editor is powerful and opaque; many users never build one. Raycast's store-first model overtook it for discovery.

## Lessons for swaypplet
- Take the Script Filter contract: a provider is a process that emits rows as structured data; elephant's menus already approximate it.
- Take fallback searches for the no-results state (see [fallback-searches](fallback-searches.md)).
- Adapt the file buffer as multi-select for clipboard and file rows, only if a concrete action on N items exists.

## Sources
- https://www.alfredapp.com/help/features/universal-actions/ — 60+ default actions, item-then-action [verified via search summary]
- https://www.alfredapp.com/help/workflows/actions/file-buffer/ — file buffer [verified via search summary]
- https://www.alfredapp.com/help/workflows/inputs/script-filter/json/ — Script Filter JSON [memory]
