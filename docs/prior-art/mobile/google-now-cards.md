---
name: Google Now cards
slug: google-now-cards
domain: mobile
kind: product
platform: Android
vendor: Google
years: 2012–2016
status: discontinued
tags: [proactive, cards, context, prediction, assistant]
relevance: medium
---

## What it is
Launched with Android 4.1 (2012), Google Now showed **cards** predicted from time, location and the user's Google data: commute time before you left, a boarding pass at the airport, a package arriving, sports scores. It lived one swipe left of the home screen (Now Launcher, 2014). Google phased out the name in 2016, replacing the cards with the Feed (later Discover) and the conversational Assistant.

## What was new
- **Information before the question**: the phone offers the commute time without being asked, at the right moment.
- The card as the unit: one fact, one action, dismissible, with "not interested" feedback.

## What went wrong / limits
- It turned into a news feed: the useful personal cards shrank, and Discover became content recommendations.
- It needed very broad access to personal data (email, location history).
- Predictions were uneven; a card that is wrong half the time teaches users to ignore all of them.

## Lessons for swaypplet
- **Avoid prediction in the shell's chrome**: the one-person, local shell has no data to predict from, and a wrong guess costs attention (P1).
- **Adapt "right card at the right moment" only with hard triggers**: a card after screen sharing ends (item 3) is a certain event, not a guess, and that is the version worth building.

## Sources
- https://en.wikipedia.org/wiki/Google_Now — history, replacement by Feed and Assistant [verified via search summary]
- https://www.howtogeek.com/789825/googles-first-assistant-the-death-of-google-now/ — decline [verified via search summary]
