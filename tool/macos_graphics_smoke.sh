#!/bin/bash
set -euo pipefail
app_path="${1:?Pass the built SnapPin.app path}"
report_dir="${2:?Pass a report directory}"
mkdir -p "$report_dir"
executable="$app_path/Contents/MacOS/SnapPin"
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
for fixture in missing-device missing-queue; do
  output="$report_dir/graphics-$fixture.log"
  SNAPPIN_GRAPHICS_SMOKE="$fixture" "$executable" >"$output" 2>&1 &
  smoke_pid=$!
  fixture_pid="$smoke_pid"
  finished=0
  for ((attempt=0; attempt<15; attempt++)); do
    if ! kill -0 "$smoke_pid" 2>/dev/null; then
      result=0
      wait "$smoke_pid" || result=$?
      smoke_pid=''
      [[ "$result" -eq 78 ]]
      finished=1
      break
    fi
    sleep 1
  done
  [[ "$finished" -eq 1 ]]
  # Native logs are written synchronously; the stderr tee may still be draining
  # when the expected early exit occurs. Check only this fixture's process.
  native_log="$HOME/Library/Application Support/com.snappin.flutter/logs/snappin-native-$(date +%F).log"
  observed="$report_dir/graphics-$fixture-native.log"
  grep "pid=$fixture_pid " "$native_log" >"$observed"
  grep -q 'native-graphics-unavailable' "$observed"
  grep -q 'native-graphics-smoke-ready.*engineCreated=false visible=false buttons=2' "$observed"
  if grep -Eq 'native-plugins-registered|native-main-engine-run-start' "$observed"; then
    cat "$output"
    echo 'Unsupported graphics unexpectedly entered Flutter startup.' >&2
    exit 1
  fi
done
echo 'Missing Metal device and command queue both report failure without starting Flutter or leaving a process.'
