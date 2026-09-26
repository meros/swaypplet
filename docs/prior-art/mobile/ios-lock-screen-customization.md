---
name: iOS 16 Lock Screen customization
slug: ios-lock-screen-customization
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2022–present
status: current
tags: [lock-screen, wallpaper, depth, typography, widgets, personalization]
relevance: medium
---

## What it is
iOS 16 made the lock screen a set of editable "posters": a wallpaper plus a clock in a chosen font and colour, a row of small widgets and a notification stack at the bottom. The **depth effect** segments the photo's subject and draws it *over* the clock. Several lock screens can be saved, swiped between and linked to Focus modes.

## What was new
- **Depth effect**: on-device segmentation puts the subject in front of the clock, the wallpaper and UI sharing one space.
- **The clock colour defaults to a colour taken from the wallpaper**, with the choice limited to fonts and colours that stay legible.
- Notifications moved to the bottom and roll up into a count, so the photo stays visible.
- Lock screens tied to a Focus: picking a lock screen switches the mode.

## What went wrong / limits
- The depth effect switches off as soon as widgets overlap the subject, which users found arbitrary.
- The rolled-up notification count at the bottom hid notifications; many users switched back to list view.
- Editing takes a long press and several screens, and is hard to discover.

## Lessons for swaypplet
- **Adapt "clock colour from the wallpaper, with legibility locked"**: the lock clock could take the tint's primary hue at a fixed tone, the same rule as §2.2.
- **Take the bottom count as an option only**: the summary row planned for the lock screen (roadmap item 3) is the right size of that idea.

## Sources
- https://www.macrumors.com/guide/ios-16-lock-screen/ — fonts, colours, widgets, depth effect [verified via search summary]
- https://support.apple.com/guide/iphone/create-a-custom-lock-screen-iph4d0e6c351/ios — link to Focus, editing [verified via search summary]
