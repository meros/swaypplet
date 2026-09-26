---
name: fzf matching algorithm
slug: fzf-algorithm
domain: launchers
kind: pattern
platform: cross
vendor: Junegunn Choi
years: 2013–present
status: current
tags: [fuzzy matching, smith-waterman, scoring, bonuses, greedy]
relevance: high
---

## What it is
The scorer in fzf, the most used fuzzy finder. Two algorithms: V1 finds a match greedily in O(n); V2, the default, is a modified Smith–Waterman local alignment that finds the best-scoring occurrence in O(nm).

## What was new
- **Explicit constants**: match 16; gap start −3, gap extension −1; bonus at a word boundary 8, after whitespace 10, after a delimiter 9, camelCase or letter/number transition 7, consecutive 4; the first pattern character's bonus doubled.
- Tuned so a boundary bonus is cancelled by a gap of about 8 characters.
- Extended search syntax: `'exact`, `^prefix`, `suffix$`, `!negate`, `|` OR, space AND.

## What went wrong / limits
- V1's greedy pass can miss the best alignment; V2 costs more on long strings, so fzf falls back to V1 on very long items.
- Scores are per string; nothing about use frequency.

## Lessons for swaypplet
- The local narrowing pass ("every query word in the name") could score with fzf-style bonuses so "vsc" puts Visual Studio Code above an app with v, s and c scattered. Use a crate rather than write it (see [nucleo-matcher](nucleo-matcher.md)).

## Sources
- https://github.com/junegunn/fzf/blob/master/src/algo/algo.go — V1/V2, constants [verified]
