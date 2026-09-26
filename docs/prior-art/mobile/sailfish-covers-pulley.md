---
name: Sailfish OS covers and pulley menus
slug: sailfish-covers-pulley
domain: mobile
kind: product
platform: other
vendor: Jolla
years: 2013–present
status: niche
tags: [gestures, active-covers, cover-actions, pulley-menu, remorse-timer, linux, qt]
relevance: medium
---

## What it is
Sailfish OS, by Jolla (founded by former Nokia MeeGo staff), continues the N9's edge-swipe model. Running apps are **active covers** on Home: small live tiles that show a summary and one or two **cover actions** (play/pause, new note). Inside apps, a **pulley menu** hides page actions above the top of the page: you drag the page down and release on the item.

## What was new
- **Active covers**: the running app's thumbnail is a designed mini-UI, not a screenshot, with actions that work without opening the app.
- **Pulley menu**: a glow at the page edge signals the menu; actions are reached with the same drag you use to scroll, one-handed.
- **Remorse timer**: destructive actions run after a few seconds' countdown you can tap to cancel, instead of a confirmation dialog.

## What went wrong / limits
- Niche: few devices, an Android compatibility layer for apps, and no mass market.
- Pulley menus are invisible past the glow; users miss them.
- Cover actions are available only to native apps.

## Lessons for swaypplet
- **Take the remorse timer**: "Clearing 12 notifications, undo" with a short countdown beats a dialog, and fits P10's stand-down thinking.
- **Adapt active covers**: Super+Tab tiles could carry one action (mute, pause) for the window's app where MPRIS or similar gives it.

## Sources
- https://sailfishos.org/design/gestures/ — gesture model [verified via search summary]
- https://www.sitepoint.com/silica-components-user-interfaces-sailfish-os/ — pulley menus, covers [verified via search summary]
- https://sailfishos.org/develop/docs/silica/ — remorse items [memory]
