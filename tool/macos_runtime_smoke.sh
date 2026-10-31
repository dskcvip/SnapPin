#!/bin/bash
set -euo pipefail
app_path="${1:?Pass the built SnapPin.app path}"
report_dir="${2:?Pass a report directory}"
mkdir -p "$report_dir"
executable="$app_path/Contents/MacOS/SnapPin"
dart_log="$HOME/Library/Application Support/com.snappin.flutter/logs/snappin-$(date +%F).log"
smoke_pid=''
cleanup() {
  if [[ -n "$smoke_pid" ]]; then
    kill "$smoke_pid" 2>/dev/null || true
    wait "$smoke_pid" 2>/dev/null || true
  fi
  log_dir="$HOME/Library/Application Support/com.snappin.flutter/logs"
  if [[ -d "$log_dir" ]]; then cp "$log_dir"/*.log "$report_dir/" 2>/dev/null || true; fi
}
trap cleanup EXIT
for mode in normal shutdown; do
  SNAPPIN_RUNTIME_SMOKE="$mode" "$executable" >"$report_dir/runtime-$mode.log" 2>&1 &
  smoke_pid=$!
  finished=0
  for ((attempt=0; attempt<90; attempt++)); do
    if ! kill -0 "$smoke_pid" 2>/dev/null; then
      wait "$smoke_pid"
      smoke_pid=''
      finished=1
      break
    fi
    sleep 1
  done
  if [[ "$finished" -ne 1 ]]; then
    kill "$smoke_pid" 2>/dev/null || true
    cat "$report_dir/runtime-$mode.log"
    echo "Runtime smoke timed out: $mode" >&2
    exit 1
  fi
  if [[ "$mode" == normal ]]; then
    grep -q 'runtime-smoke-shadow-ready' "$dart_log"
    grep -q 'runtime-smoke-ocr-responsive' "$dart_log"
    grep -q 'runtime-smoke-ocr-layout-ready' "$dart_log"
    grep -q 'runtime-smoke-ocr-busy-ready' "$dart_log"
    grep -q 'runtime-smoke-ocr-error-ready' "$dart_log"
    grep -q '\[runtime-smoke-ready\]' "$dart_log"
  else
    grep -q 'runtime-smoke-shutdown-start' "$dart_log"
  fi
done
echo 'Native OCR layout, responsiveness, invalid input, shadow and shutdown passed.'
