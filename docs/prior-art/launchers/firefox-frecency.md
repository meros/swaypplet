---
name: Firefox frecency
slug: firefox-frecency
domain: launchers
kind: pattern
platform: cross
vendor: Mozilla
years: 2008–present
status: current
tags: [ranking, frecency, exponential decay, visit weights, interactions]
relevance: high
---

## What it is
The ranking of the Firefox address bar: each page's score combines how often and how recently it was visited, weighted by how it was visited. The word "frecency" comes from here (Firefox 3, 2008).

## What was new
- **Weighted visits**: typed or bookmarked visits weigh high, link clicks medium, redirects, reloads and framed visits low; a "very high" bucket for bookmarked/typed pages with real interaction.
- **Sampled, decayed**: only the last N visits count, each decayed exponentially with a ~30-day half-life: `score = Σ weight · e^(−λ(t_ref − t_visit))`, λ = ln 2 / 30.
- **Stored as a date**: the value kept is the day the score would decay to 1, `t_ref + ln(score)/λ`, so it never needs recomputing as time passes and can be compared directly.
- **Interactions** (recent): a visit counts as "interesting" if viewed long enough or with many keypresses; unpaired interactions become virtual visits.

## What went wrong / limits
- Older versions recomputed scores in bulk and were a known source of jank on large histories; the decay-to-date trick fixed that.
- Many prefs and buckets; hard to reason about.

## Lessons for swaypplet
- swaypplet's `frecency.rs` already implements the continuous core (7-day half-life, lazy decay). Adopt the **stored-as-date** form: store `t_ref + ln(score)/λ` and sorting needs no decay math at read time.
- Adapt **weights by how an item was launched**: an explicit query + Enter should count more than a click on an empty-state suggestion, which otherwise feeds on itself.

## Sources
- https://firefox-source-docs.mozilla.org/browser/urlbar/ranking.html — buckets, sampling, 30-day half-life, formula, interactions [verified]
