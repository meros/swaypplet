#!/bin/sh
# Show or hide the launcher. Same contract as swaypplet-toggle, USR2 instead of
# USR1, and the same reason for accepting the wrapper's name in comm.
PID=$(cat "${XDG_RUNTIME_DIR:-/tmp}/swaypplet.pid" 2>/dev/null)
case "$(cat /proc/"$PID"/comm 2>/dev/null)" in
  swaypplet | .swaypplet*) kill -USR2 "$PID" ;;
  *) swaypplet launcher & ;;
esac
