#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
line='['
for arg in "$@"; do line="$line<$arg>"; done
printf '%s]\n' "$line" >> "$root/calls.log"
pwd > "$root/query-cwd"
if [ "$#" -ne 4 ] || [ "$1" != view ] || [ "$3" != version ] || [ "$4" != --json ]; then
  exit 90
fi
response="$root/responses/$2"
if [ -f "$response.delay" ]; then sleep "$(cat "$response.delay")"; fi
if [ -f "$response.exit" ]; then
  printf 'registry request failed token=do-not-expose-this\n' >&2
  exit "$(cat "$response.exit")"
fi
cat "$response.json"
