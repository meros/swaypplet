#!/usr/bin/env bash
# Create the labels a report and a crash are filed with.
# Idempotent (--force updates colour and text). Run once per repository:
#   dev/autofix/labels.sh [owner/repo]
set -euo pipefail
REPO="${1:-meros/swaypplet}"
label() { gh label create "$1" --repo "$REPO" --color "$2" --description "$3" --force; }
label report          1d76db "Filed from the desktop with swaypplet report"
label crash           b60205 "Filed by swaypplet crash-report when the service failed"
