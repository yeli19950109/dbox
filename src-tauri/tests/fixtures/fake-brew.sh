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
    printf 'Homebrew 4.6.0\n'
    ;;
  --prefix)
    printf '%s\n' "$fixture_dir/prefix"
    ;;
  config)
    printf 'HOMEBREW_VERSION: 4.6.0\nCPU: arm64\n'
    ;;
  info)
    if [ -f "$fixture_dir/info.stderr" ]; then
      command cat "$fixture_dir/info.stderr" >&2
    fi
    command cat "$fixture_dir/info.json"
    if [ -f "$fixture_dir/info.exit" ]; then
      exit "$(command cat "$fixture_dir/info.exit")"
    fi
    ;;
  list)
    if [ -f "$fixture_dir/lists/$4.txt" ]; then
      command cat "$fixture_dir/lists/$4.txt"
    fi
    if [ -f "$fixture_dir/list.exit" ]; then
      exit "$(command cat "$fixture_dir/list.exit")"
    fi
    ;;
  outdated)
    if [ -f "$fixture_dir/outdated.stderr" ]; then
      command cat "$fixture_dir/outdated.stderr" >&2
    fi
    command cat "$fixture_dir/outdated.json"
    if [ -f "$fixture_dir/outdated.exit" ]; then
      exit "$(command cat "$fixture_dir/outdated.exit")"
    fi
    ;;
  upgrade)
    printf 'fake brew never upgrades packages\n' >&2
    exit 90
    ;;
  *)
    printf 'unexpected fake brew command\n' >&2
    exit 91
    ;;
esac
