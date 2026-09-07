#!/bin/sh

mode="$1"

case "$mode" in
  streams)
    printf 'stdout-one\n'
    printf 'stderr-one\n' >&2
    printf '\033[31mstdout-two\033[0m\n'
    ;;
  live-gated)
    printf 'fixture-started\n'
    while [ ! -f "$2" ]; do sleep 0.02; done
    printf 'fixture-finished\n' >&2
    ;;
  fail)
    printf 'command failed\n' >&2
    exit 7
    ;;
  invalid-utf8)
    printf '\377bad-byte\n'
    ;;
  long)
    index=0
    while [ "$index" -lt 3000 ]; do
      printf 'long-output-%04d-abcdefghijklmnopqrstuvwxyz\n' "$index"
      index=$((index + 1))
    done
    ;;
  args)
    shift
    for value in "$@"; do
      printf '<%s>\n' "$value"
    done
    ;;
  secret)
    printf 'TOKEN=%s\n' "$TOKEN"
    printf 'raw:%s\n' "$TOKEN"
    ;;
  sleep-tree)
    prefix="$2"
    printf '%s\n' "$$" > "${prefix}.parent"
    sleep 30 &
    child="$!"
    printf '%s\n' "$child" > "${prefix}.child"
    wait
    ;;
  *)
    printf 'unknown fixture mode: %s\n' "$mode" >&2
    exit 64
    ;;
esac
