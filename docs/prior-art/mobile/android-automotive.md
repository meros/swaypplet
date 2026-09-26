---
name: Android Automotive OS
slug: android-automotive
domain: mobile
kind: product
platform: Android
vendor: Google and car makers
years: 2017–present
status: current
tags: [car, templates, safety, restricted-ui, oem-theming]
relevance: low
---

## What it is
A full Android build that runs the car's own head unit (Polestar 2, 2020, first with Google services; Volvo, GM, Renault and others since). Unlike Android Auto, it is not projected from a phone. Third-party apps are limited to media, messaging, navigation and point-of-interest categories, and must use **car app templates** that the system renders, with limits applied while driving.

## What was new
- **Templates as a safety policy**: apps give data and the system draws it, enforcing limits (list length, scrolling, text entry off while driving).
- **Restriction by vehicle state**: UI capabilities change as the car moves or parks.
- OEMs theme the system freely over the same templates (Volvo and GM look different on the same code).

## What went wrong / limits
- Fragmentation: every OEM ships its own version and updates slowly.
- GM dropping CarPlay and Android Auto in favour of it (from 2024 electric models) drew strong criticism from buyers.

## Lessons for swaypplet
- **Take "data from the source, drawing by the host"**: any future plugin or segment source should hand over data, never widgets, so tokens and contrast rules always apply (design-system §7 enforcement).
- **Adapt state-based restrictions**: surfaces could reduce what they show while screen sharing, the same trigger roadmap item 3 uses.

## Sources
- https://en.wikipedia.org/wiki/Android_Automotive — history, Polestar 2, OEMs [memory]
- https://developer.android.com/training/cars/apps — car app templates and driving restrictions [memory]
