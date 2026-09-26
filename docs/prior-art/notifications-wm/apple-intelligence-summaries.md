---
name: Apple Intelligence notification summaries
slug: apple-intelligence-summaries
domain: notifications-wm
kind: feature
platform: iOS
vendor: Apple
years: 2024–present
status: current
tags: [ai, llm, summaries, misinformation, trust, priority]
relevance: medium
---

## What it is
In iOS 18.1 (late 2024) an on-device language model rewrote stacks of notifications into one-line summaries on the Lock Screen. In December 2024 the BBC complained that a summary of its alerts falsely said Luigi Mangione had shot himself; other outlets reported similar errors. iOS 18.3 (January 2025) disabled summaries for news and entertainment apps, set summaries in italics, added a per-app off switch on the Lock Screen and labelled the feature beta. iOS 18.4 added "Prioritize Notifications", which floats model-chosen items to the top.

## What was new
Summarising a group rather than showing its latest item; useful for chatty group chats.

## What went wrong / limits
A summary sits in the same visual slot as the sender's own text, so the model's errors were read as the publisher's words. Compressing already-short texts removes the negations and qualifiers that carry meaning. The fix was visual distinction and opt-out, i.e. admitting the output is a different kind of content.

## Lessons for swaypplet
- Avoid: generating or rewriting notification text; a group summary should be mechanical (count, senders, latest title) and verifiable.
- Take: anything the shell synthesises must look different from what the sender wrote.
- Adapt: priority ordering can be rule-based (urgency, people categories) instead of model-based.

## Sources
- https://techcrunch.com/2025/01/16/apple-pauses-ai-notification-summaries-for-news-after-generating-false-alerts — pause, BBC complaint. [verified via search summary]
- https://www.cnbc.com/2025/01/16/apple-disables-ai-notifications-for-news-in-its-beta-iphone-software.html — italics, per-app disable, beta label. [verified via search summary]
- https://techcrunch.com/2025/02/21/ios-18-4-will-bring-apple-intelligence-powered-priority-notifications — priority notifications. [verified via search summary]
