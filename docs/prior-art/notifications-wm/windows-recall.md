---
name: Windows Recall
slug: windows-recall
domain: notifications-wm
kind: feature
platform: Windows
vendor: Microsoft
years: 2024–present
status: current
tags: [screenshots, ai, privacy, security, opt-in, rollout]
relevance: medium
---

## What it is
Announced May 2024 for Copilot+ PCs: a screenshot of the screen every few seconds, OCR'd and indexed so the user can search everything they have seen. Kevin Beaumont showed the preview stored its database in plaintext readable by any user process, so infostealers could lift it. Microsoft made it opt-in, encrypted behind Windows Hello, and delayed it several times; it reached general release in 2025.

## What was new
Total recall of screen history by semantic search.

## What went wrong / limits
Designed on by default and without threat modelling: a complete record of passwords, messages and banking on screen, in an unencrypted file. The feature's value is proportional to its danger. Trust lost at announcement shaped every later news story even after the fixes.

## Lessons for swaypplet
- Avoid: persisting screen captures. Super+Tab's workspace pictures and the overview's first frame must stay in memory, never on disk (the same stance as the rejected clipboard persistence).
- Take: capture-based features start opt-in and say where data lives.

## Sources
- https://doublepulsar.com/microsoft-recall-on-copilot-pc-testing-the-security-and-privacy-implications-ddb296093b6c — Beaumont's findings. [verified via search summary]
- https://www.bleepingcomputer.com/news/microsoft/microsoft-delays-windows-recall-amid-privacy-and-security-concerns/ — delays. [verified via search summary]
- https://www.theregister.com/2024/06/07/microsoft_recall_changes — opt-in and Windows Hello. [verified via search summary]
- General release in 2025. [memory]
