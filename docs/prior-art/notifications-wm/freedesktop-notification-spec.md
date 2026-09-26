---
name: Desktop Notifications Specification (org.freedesktop.Notifications)
slug: freedesktop-notification-spec
domain: notifications-wm
kind: protocol
platform: cross
vendor: freedesktop.org (originally Galago project)
years: 2004–present (1.3, 2024-08-18)
status: current
tags: [dbus, notifications, urgency, hints, actions, persistence, activation-token]
relevance: high
---

## What it is
The D-Bus interface every Linux notification daemon implements: `Notify`, `CloseNotification`, `GetCapabilities`, `GetServerInformation`, and the signals `NotificationClosed`, `ActionInvoked` and (1.3) `ActivationToken`. Clients pass a summary, body, icon, actions and a dictionary of hints; the server decides how to show them. swaypplet is such a server.

## What was new
A deliberately small contract with capability negotiation: the server advertises `actions`, `body-markup`, `persistence`, `sound`, `action-icons` and so on, and clients degrade. Three urgency levels (0 low, 1 normal, 2 critical); the spec says critical notifications "should not automatically expire". Hints carry `category` (`im.received`, `email.arrived`, `device.added`, ...), `desktop-entry`, `transient` (skip persistence), `resident` (stay after an action), `image-data`, `sound-name`, `suppress-sound`. `replaces_id` lets a client update a notification in place.

## What went wrong / limits
No grouping, threading, inline reply, per-channel identity or interruption levels beyond urgency; every server invents these (GNOME moved to the XDG portal, KDE added private hints, Canonical `x-canonical-private-synchronous` for OSDs). Clients are unreliable about `desktop-entry` and `category`, so grouping by app has to fall back on `app_name` heuristics. Hints are optional on both sides.

## Lessons for swaypplet
- Take: map urgency onto interruption levels (low: list only, no popup; critical: breaks through quiet modes, no timeout) instead of treating it as colour.
- Take: `replaces_id` and `transient` are the cheap ways to stop floods and keep OSD-like chatter out of history; honour both.
- Adapt: group by `desktop-entry`, fall back to `app_name`, and treat `category` as a hint for thread grouping, never as trusted.

## Sources
- https://specifications.freedesktop.org/notification/latest-single/ — methods, signals, hints, capabilities, critical never expires, version 1.3 date. [verified]
- https://specifications.freedesktop.org/notification/1.2/hints.html — hint semantics (resident, transient). [verified]
- https://blogs.gnome.org/shell-dev/2024/04/23/notifications-46-and-beyond/ — GNOME moving investment to the portal API. [verified]
