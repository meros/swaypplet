---
name: nucleo (Rust fuzzy matcher)
slug: nucleo-matcher
domain: launchers
kind: project
platform: cross
vendor: Pascal Kuthe (Helix editor)
years: 2023–present
status: current
tags: [rust, fuzzy matching, smith-waterman, unicode, parallel, crate]
relevance: high
---

## What it is
A Rust fuzzy-matching library written for the Helix editor's pickers, split into `nucleo-matcher` (the scorer) and `nucleo` (a parallel, incremental matcher over a growing item list).

## What was new
- fzf-compatible scoring (Smith–Waterman with affine gaps and fzf's bonuses) with a faster implementation; reported several times faster than skim and fuzzy-matcher.
- Unicode-aware: case folding and optional normalisation so "e" matches "é".
- Items can be streamed in while matching runs on a thread pool, and results are snapshotted per frame.

## What went wrong / limits
- API still pre-1.0; `nucleo` (the parallel layer) is heavier than a launcher needs.

## Lessons for swaypplet
- Take `nucleo-matcher` for the local narrowing pass and window search: a pure-Rust, allocation-light scorer, same scoring users know from fzf. Normalisation matters for Swedish app and file names (å, ä, ö).

## Sources
- https://github.com/helix-editor/nucleo — crate, design, benchmarks [memory]
