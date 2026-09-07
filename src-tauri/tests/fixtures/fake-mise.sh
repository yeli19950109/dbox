#!/bin/sh
set -eu
base=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
# Require the same explicit context in discovery, checks and executed plans.
[ "$1" = '--cd' ]
[ "$2" = "$base/home with spaces" ]
shift 2
printf '%s\n' "$*" >> "$base/calls.log"
case "$1" in
  version) printf '2026.9.1 macos-arm64 (fixture)\n' ;;
  ls)
    file=installed
    for arg do
      case "$arg" in --global) file=global ;; --current) file=current ;; esac
    done
    if [ -f "$base/$file.fail" ]; then
      printf 'offline TOKEN=secret-fixture\n' >&2
      exit 1
    fi
    cat "$base/$file.json"
    ;;
  tool)
    [ "$2" = '--json' ] && [ "$3" = '--' ]
    key=$(printf '%s' "$4" | tr '/:' '__')
    if [ -f "$base/$key.fail" ]; then
      printf 'plugin is not installed SECRET=plugin-secret\n' >&2
      exit 1
    fi
    cat "$base/$key.json"
    ;;
  latest)
    [ "$2" = '--' ]
    key=$(printf '%s' "$3" | tr '/:' '__')
    if [ -f "$base/$key.delay" ]; then sleep 1; fi
    if [ -f "$base/$key.fail" ]; then
      printf 'offline TOKEN=secret-fixture\n' >&2
      exit 1
    fi
    cat "$base/$key.latest"
    ;;
  upgrade)
    if [ "$2" = '--help' ]; then
      if [ -f "$base/old-version" ]; then printf 'upgrade\n'; else printf 'upgrade --no-prune\n'; fi
    else
      [ "$2" = '--no-prune' ] && [ "$3" = '--yes' ] && [ "$4" = '--' ]
      [ "$#" = 5 ]
      if [ -f "$base/after-installed.json" ]; then
        cp "$base/after-installed.json" "$base/installed.json"
        cp "$base/after-global.json" "$base/global.json"
        cp "$base/after-global.json" "$base/current.json"
      fi
    fi
    ;;
  *) exit 64 ;;
esac
