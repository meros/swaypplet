---
name: Browser omnibox and awesomebar
slug: browser-omnibox
domain: launchers
kind: pattern
platform: cross
vendor: Google (Chrome), Mozilla (Firefox)
years: 2008–present
status: current
tags: [unified input, inline autocomplete, keyword search, tab to search, providers]
relevance: high
---

## What it is
Firefox 3's awesomebar and Chrome's omnibox (both 2008) merged the URL field and search box. Results come from several providers (history, bookmarks, open tabs, search suggestions, shortcuts) merged into one list with one default match.

## What was new
- **Inline autocomplete**: the top match completes the typed text in place, selected, so Enter goes there and typing on overwrites it. Only a high-confidence match may autofill.
- **Keyword search / Tab to search**: typing a site's keyword then Tab scopes the query to that site's search.
- **Provider architecture**: Chrome's AutocompleteController runs sync providers immediately and async ones later, re-sorting without moving the default match under the cursor.
- Chrome's "shortcuts" provider remembers text→URL choices (the same idea as [adaptive-input-history](adaptive-input-history.md)).

## What went wrong / limits
- Search suggestions send every keystroke to the search engine.
- Result reordering as async providers land causes mis-clicks; both browsers added rules to keep the top row stable.

## Lessons for swaypplet
- Take the stability rule: once elephant's rows land, the selected row must not move from under the cursor within ~100 ms of Enter.
- Take inline completion only for a strong learned match.

## Sources
- https://www.chromium.org/omnibox-history-provider/ — HistoryQuick scoring [memory]
- https://firefox-source-docs.mozilla.org/browser/urlbar/ranking.html — urlbar ranking [verified]
