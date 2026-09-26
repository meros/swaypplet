---
name: Quicksilver
slug: quicksilver
domain: launchers
kind: product
platform: macOS
vendor: Blacktree (Nicholas Jitkoff), later open source
years: 2003–present
status: niche
tags: [verb-object, three panes, catalog, mnemonics, plugins]
relevance: medium
---

## What it is
A keyboard "graphical shell" for macOS started by Nicholas Jitkoff in 2003, open-sourced in 2007 and maintained by volunteers since. Three panes: object, action, optional indirect object, like a sentence.

## What was new
- **Verb-object grammar**: choose a file, Tab, choose "Email to", Tab, choose a contact. Any action applies to any object of a compatible type.
- **Mnemonics**: it learns the abbreviation you typed for the item you picked and ranks it first for that abbreviation next time.
- Comma-trick: collect several objects into one pane before acting.
- Defined the genre; much of it later landed in Spotlight, Alfred and LaunchBar.

## What went wrong / limits
- Jitkoff left for Google in 2007; development stalled, plugins broke across macOS releases, and stability suffered.
- The three-pane grammar has a learning curve most users never climbed; later launchers hid the verb behind an action menu.

## Lessons for swaypplet
- Take the type system: rows carry a kind (app, file, text, window) and actions are offered by kind, not per provider.
- Avoid exposing the grammar as always-on panes; reveal actions on demand (see [action-panel](action-panel.md)).

## Sources
- https://en.wikipedia.org/wiki/Quicksilver_(software) — history, three-pane model [verified via search summary]
- https://qsapp.com/manual/ — mnemonics, comma trick [memory]
