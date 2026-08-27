#!/bin/sh

mode="$1"

case "$mode" in
  record)
    log_file="$2"
    item_id="$3"
    outcome="$4"
    active_dir="$5"
    overlap_file="$6"
    if ! mkdir "$active_dir" 2>/dev/null; then
      printf '%s\n' "$item_id" >> "$overlap_file"
    fi
    printf '%s\n' "$item_id" >> "$log_file"
    sleep 0.01
    rmdir "$active_dir" 2>/dev/null || true
    if [ "$outcome" = "fail" ]; then
      printf 'fixture failure for %s\n' "$item_id" >&2
      exit 7
    fi
    ;;
  hold)
    started_file="$2"
    release_file="$3"
    item_id="$4"
    printf '%s\n' "$item_id" > "$started_file"
    while [ ! -f "$release_file" ]; do
      sleep 0.02
    done
    ;;
  *)
    printf 'unknown queue fixture mode: %s\n' "$mode" >&2
    exit 64
    ;;
esac
