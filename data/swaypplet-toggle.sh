#!/bin/sh
# Show or hide the panel, from a keybind or a bar click.
#
# The pid file is written by the running panel. The comm check is what keeps a
# recycled pid from being sent a signal whose default disposition is fatal, and
# it has to accept the wrapper's name: nixpkgs wraps the binary, so the live
# panel's executable is .swaypplet-wrapped and comm, which is the executable
# filename cut to 15 characters, reads ".swaypplet-wrap". Testing for the bare
# name matched nothing and every toggle silently started a second panel.
PID=$(cat "${XDG_RUNTIME_DIR:-/tmp}/swaypplet.pid" 2>/dev/null)
case "$(cat /proc/"$PID"/comm 2>/dev/null)" in
  swaypplet | .swaypplet*) kill -USR1 "$PID" ;;
  *) swaypplet & ;;
esac
