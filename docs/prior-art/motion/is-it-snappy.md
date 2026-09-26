---
name: Is It Snappy?
slug: is-it-snappy
domain: motion
kind: project
platform: iOS
vendor: Chad Austin
years: 2017–present
status: niche
tags: [input-latency, high-speed-camera, end-to-end, photon]
relevance: medium
---

## What it is
An iOS app that uses the iPhone's 240 fps camera (about 4 ms resolution) to measure end-to-end latency: film the input and the screen, then mark the frame of the key press and the frame the screen changes.

## What was new
End-to-end, photon-level measurement with a phone instead of lab equipment. Published results: an NES on a CRT near zero, NES Classic on a Dell LCD 87 ms, on a Samsung TV about 170 ms; Notepad on Windows 10 70.8 ms, VS Code 116.7 ms; a Mac's built-in keyboard about 40 ms slower than an external one.

## What went wrong / limits
Manual frame marking is slow and noisy; not automatable in CI.

## Lessons for swaypplet
- Take: a one-off photon measurement of launcher open (key press to first visible glass frame) on the real laptop, to calibrate what the headless gate's numbers mean.

## Sources
- https://isitsnappy.com/ — method, 240 fps, published measurements [verified]
