---
name: Two-stage lock screen (curtain, then credentials)
slug: lock-screen-curtain
domain: shells
kind: pattern
platform: cross
vendor: Microsoft, GNOME, Apple (contrast)
years: 2012–present
status: current
tags: [lock-screen, greeter, curtain, notifications, privacy]
relevance: high
---

## What it is
Windows 8 onward and GNOME Shell (the "shield", 3.6+) show a curtain first: a large clock, date, wallpaper and a few status items (notifications count, media, battery). A key press, click or upward swipe lifts it to reveal the credential field. macOS goes straight to the credential field over the blurred wallpaper, with the clock above it (Sonoma 2023 and later use the full wallpaper and a large clock).

## What was new
- The curtain separates ambient information from authentication, and doubles as a touch affordance (swipe up).
- Windows 10's lock screen could show a detailed status from one app (calendar, mail) and brief status from several; Windows Spotlight rotates wallpapers.
- GNOME shows notifications on the shield only as counts per app, with a privacy setting for content.

## What went wrong / limits
- On desktops the curtain is one extra step every unlock; typing on the curtain in Windows and GNOME goes straight to the password field to hide this.
- Lock-screen content is a privacy surface; showing notification text leaks it to anyone passing by.
- Windows 11's lock-screen weather and news widgets drew the same criticism as the Widgets board.

## Lessons for swaypplet
- Take: typing on the curtain goes into the password field; fingerprint or face works without lifting it (AUTH_CARD already shares the card).
- Take: ROADMAP item 3's summary row on the lock screen as counts per app, no content by default.
- Avoid: feeds or weather on the lock screen.

## Sources
- https://help.gnome.org/users/gnome-help/stable/shell-exit.html — GNOME lock screen behaviour [memory]
- https://en.wikipedia.org/wiki/Windows_8 — lock screen introduction [memory]
- https://support.microsoft.com/en-us/windows/personalize-your-lock-screen-in-windows — lock screen status and Spotlight [memory]
