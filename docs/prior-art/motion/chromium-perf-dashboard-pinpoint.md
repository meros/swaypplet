---
name: Chromium perf bots, dashboard and Pinpoint
slug: chromium-perf-dashboard-pinpoint
domain: motion
kind: project
platform: cross
vendor: Google
years: 2012–present
status: current
tags: [ci, perf-regression, bisect, anomaly-detection, sheriff]
relevance: medium
---

## What it is
Chrome runs benchmarks (Telemetry, later crossbench, including rendering and smoothness stories) on a fleet of dedicated devices after every batch of commits. Results go to a perf dashboard that detects step changes; a rotating "perf sheriff" triages alerts, and Pinpoint bisects a regression to one commit by re-running the benchmark on both sides.

## What was new
Perf is gated after merge rather than before: noisy metrics cannot block every CL, so the system finds the step change statistically and bisects it automatically. A regression postmortem template and "my CL caused a regression" guide make the process routine.

## What went wrong / limits
Needs a large dedicated device fleet; alert noise is a constant cost and sheriffs close many alerts as noise.

## Lessons for swaypplet
- Take: keep a history of frame-bench outputs per commit (a CSV in the repo or a git note) so a slow drift that stays under the gate's fixed limits still shows.
- Take: a tiny Pinpoint: `git bisect run dev/frame-bench.sh --gate` works once the gate is stable enough to be bisectable.

## Sources
- https://chromium.googlesource.com/chromium/src/+/HEAD/docs/speed/README.md — dashboard, bisect, regression guide, postmortem template [verified]
