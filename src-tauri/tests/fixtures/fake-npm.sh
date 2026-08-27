#!/bin/sh

fixture_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
calls_file="$fixture_dir/calls.log"

call_line='['
for argument in "$@"; do
  call_line="${call_line}<${argument}>"
done
printf '%s]\n' "$call_line" >> "$calls_file"

case "$1" in
  --version)
    printf '10.9.1\n'
    ;;
  prefix)
    printf '%s\n' "$fixture_dir/prefix"
    ;;
  root)
    printf '%s\n' "$fixture_dir/prefix/lib/node_modules"
    ;;
  ls)
    if [ -f "$fixture_dir/list.json" ]; then
      command cat "$fixture_dir/list.json"
    else
      printf '{"dependencies":{}}\n'
    fi
    if [ -f "$fixture_dir/list.exit" ]; then
      exit "$(command cat "$fixture_dir/list.exit")"
    fi
    ;;
  outdated)
    if [ -f "$fixture_dir/outdated.json" ]; then
      command cat "$fixture_dir/outdated.json"
    else
      printf '{}\n'
    fi
    if [ -f "$fixture_dir/outdated.exit" ]; then
      exit "$(command cat "$fixture_dir/outdated.exit")"
    fi
    ;;
  install)
    printf 'fake npm never installs packages\n' >&2
    exit 90
    ;;
  *)
    printf 'unexpected fake npm command\n' >&2
    exit 91
    ;;
esac
