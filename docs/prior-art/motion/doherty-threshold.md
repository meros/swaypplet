---
name: Doherty threshold (400 ms)
slug: doherty-threshold
domain: motion
kind: pattern
platform: other
vendor: IBM (Doherty and Thadani)
years: 1982
status: research
tags: [response-time, productivity, perception, 400ms]
relevance: medium
---

## What it is
Walter Doherty and Ahrvind Thadani's 1982 IBM Systems Journal paper argued that system response under about 400 ms markedly raises user productivity, against the 2-second standard of the time.

## What was new
It tied response time to throughput, not comfort: below the threshold users stop waiting and keep a continuous flow of commands. Its modern use, via "Laws of UX", is to justify animation and skeletons that make a response appear within 400 ms.

## What went wrong / limits
Mainframe-era data on transaction systems; the "addicting" framing is popularisation, not the paper. The 400 ms figure is a system response, not an animation duration, and is often misapplied as one.

## Lessons for swaypplet
- Take: the shell's ceiling for "the thing I asked for is on screen and usable" is 400 ms including animation. The `dwell` tier (500 ms) must never gate interaction; input should be accepted from the first frame of any entrance.

## Sources
- https://lawsofux.com/doherty-threshold/ — summary and origin [verified]
- https://jlelliotton.blogspot.com/p/the-economic-value-of-rapid-response.html — the paper's argument, reproduced [memory]
