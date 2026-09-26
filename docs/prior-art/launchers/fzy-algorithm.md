---
name: fzy matching algorithm
slug: fzy-algorithm
domain: launchers
kind: pattern
platform: cross
vendor: John Hawthorn
years: 2014–present
status: current
tags: [fuzzy matching, dynamic programming, optimal alignment, scoring]
relevance: medium
---

## What it is
The scorer of fzy, a small C fuzzy finder. It fills an n×m dynamic-programming matrix (like Needleman–Wunsch/Wagner–Fischer) plus a parallel "D" matrix of best matches ending at each position, and returns the optimal score.

## What was new
- **Optimal, not greedy**: fzy's author argued fzf's early greedy shortest-match did not guarantee the best ranking; fzy always computes the best alignment.
- Bonuses for consecutive matches, matches after `/ - _ space`, capitals, and after a dot; penalties for leading, inner and trailing gaps.

## What went wrong / limits
- O(nm) per candidate; fine for thousands of items, slower for millions. fzf's V2 later closed most of the quality gap.

## Lessons for swaypplet
- For a few hundred apps and windows, optimal scoring costs microseconds; there is no reason to accept greedy matching in the local pass.

## Sources
- https://github.com/jhawthorn/fzy/blob/master/ALGORITHM.md — algorithm, bonuses, comparison with fzf [verified]
