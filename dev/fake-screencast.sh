#!/usr/bin/env bash
# A stand-in for `pw-dump --monitor`, for the harness: prints the graph with
# one wlr-portal screen-cast stream in it, holds it for SWPP_SHARE_S seconds
# (default 6), removes it, and idles. Point swaypplet at it with
#   SWAYPPLET_PW_DUMP=dev/fake-screencast.sh
# so the "screen shared" path (src/services/capture.rs) runs end to end
# without a real cast. Arguments (--monitor --no-colors) are ignored.
node='{ "id": 900, "type": "PipeWire:Interface:Node", "info": { "state": "paused",
  "props": { "media.class": "Video/Source", "node.name": "xdpw-stream" } } }'
printf '[\n%s\n]\n' "$node"
sleep "${SWPP_SHARE_S:-6}"
printf '[\n{ "id": 900, "info": null }\n]\n'
exec sleep 600
