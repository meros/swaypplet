---
name: Typometer and "Typing with pleasure"
slug: typometer
domain: motion
kind: project
platform: cross
vendor: Pavel Fatin
years: 2015
status: niche
tags: [input-latency, measurement, editors, screen-capture, jitter]
relevance: high
---

## What it is
An open-source tool that synthesises key events and uses screen capture to time keystroke-to-screen-update inside the software stack. It came with an essay measuring editors on Windows and Linux.

## What was new
- Latency broken into input (keyboard 8–22 ms average), processing (anything) and output (60 Hz refresh about 8 ms average, pixel response about 4 ms).
- Measured spreads: GVim 0.9 ms average, IntelliJ zero-latency mode 2.9 ms, Notepad++ 4.3 ms, Atom 49.4 ms; IntelliJ default mode hit 83.7 ms max. Jitter mattered as much as the mean.
- Argued that even latency below conscious notice degrades typing because it disturbs the motor loop.

## What went wrong / limits
Screen capture sees the compositor's buffer, not photons; it misses display latency and compositor scheduling. Unmaintained.

## Lessons for swaypplet
- Take: a launcher latency gate. Synthesise a key into the launcher's search field on the headless sway, capture the output with `wlr-screencopy` or read swaypplet's frame log, and record key-to-committed-frame time. Report mean and max, not only mean.

## Sources
- https://pavelfatin.com/typing-with-pleasure/ — method, component budget, editor numbers [verified]
- https://github.com/pavelfatin/typometer — tool [memory]
