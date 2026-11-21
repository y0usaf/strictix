#!/bin/sh
set -eu

for f in "$@"; do
  if [ -f "$f" ]; then
    printf '%s\n' "$f"
  fi
done
