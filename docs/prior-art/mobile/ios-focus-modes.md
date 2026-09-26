---
name: iOS Focus modes
slug: ios-focus-modes
domain: mobile
kind: feature
platform: iOS
vendor: Apple
years: 2021–present
status: current
tags: [do-not-disturb, context, automation, notifications, home-screen, filters]
relevance: high
---

## What it is
Since iOS 15, Do Not Disturb generalised into named Focus modes (Work, Sleep, Driving, custom), each with allowed people and apps, chosen home-screen pages, a lock screen and, since iOS 16, Focus filters that change what apps show inside them (only the work calendar, say). Focus modes turn on by schedule, location, opening an app or detected driving, and they sync across devices.

## What was new
- **Context changes the whole shell, not only the alerts**: pages, lock screen and in-app data follow the mode.
- **Automatic triggers**: driving detection and "Smart activation" guess the mode.
- **Status shared with others**: Messages can say "X has notifications silenced", and senders may break through for urgent messages.

## What went wrong / limits
- The configuration is deep (people, apps, pages, filters, schedules per mode), and many users never set one up past Sleep.
- Cross-device sync surprised users when a Focus turned on at the wrong time on another device.
- Smart activation is opaque: the user cannot see why a mode switched on.

## Lessons for swaypplet
- **Take triggers from real signals, and show them**: roadmap item 3 (DND when sharing a screen or in fullscreen) is a Focus with automatic triggers. Show the reason on the DND tile ("On: screen shared") so it is never opaque.
- **Avoid a configuration matrix**: two or three built-in contexts with fixed rules beat a user-built rule editor for one person.
- **Adapt breakthrough**: a small allow-list (a Claude session waiting, a critical battery) that still gets through, in line with P10's stand-down table.

## Sources
- https://notificare.com/blog/2021/10/04/Revisting-Notifications-in-iOS-15/ — Focus and notification changes in iOS 15 [verified via search summary]
- https://support.apple.com/guide/iphone/set-up-a-focus-iphd6288a67f/ios — Focus setup, triggers and filters [memory]
