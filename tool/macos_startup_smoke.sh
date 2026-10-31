#!/bin/bash
set -euo pipefail

app_path="${1:?Pass the built SnapPin.app path}"
report_dir="${2:?Pass a report directory}"
mkdir -p "$report_dir"
executable="$app_path/Contents/MacOS/SnapPin"

# Both the app and every embedded framework must work on the user's Intel Mac.
lipo "$executable" -verify_arch x86_64 arm64
while IFS= read -r -d '' binary; do
  if file "$binary" | grep -q 'Mach-O'; then
    lipo "$binary" -verify_arch x86_64 arm64
  fi
done < <(find "$app_path/Contents/Frameworks" -type f -print0)

SNAPPIN_STARTUP_SMOKE=1 "$executable" >"$report_dir/console.log" 2>&1 &
smoke_pid=$!
cleanup() {
  kill "$smoke_pid" 2>/dev/null || true
  wait "$smoke_pid" 2>/dev/null || true
  log_dir="$HOME/Library/Application Support/com.snappin.flutter/logs"
  if [[ -d "$log_dir" ]]; then
    cp "$log_dir"/*.log "$report_dir/" 2>/dev/null || true
  fi
}
trap cleanup EXIT
dart_log="$HOME/Library/Application Support/com.snappin.flutter/logs/snappin-$(date +%F).log"

for ((attempt=0; attempt<45; attempt++)); do
  if ! kill -0 "$smoke_pid" 2>/dev/null; then
    exit_code=0
    wait "$smoke_pid" || exit_code=$?
    if [[ "$exit_code" -eq 0 ]] &&
       grep -q 'macos-startup-smoke-ready' "$dart_log" &&
       grep -q 'macos-tray-menu-smoke-ready.*left action=toggle-ball' "$dart_log" &&
       grep -q 'macos-tray-menu-smoke-ready.*right action=toggle-ball' "$dart_log" &&
       grep -q 'macos-menu-quit-smoke-start' "$dart_log" &&
       grep -q 'app-quit-requested.*menu' "$dart_log"; then
      echo 'Main window, hidden overlay engine, both tray callbacks, menu actions and process exit passed.'
      exit 0
    fi
    cat "$report_dir/console.log"
    echo "SnapPin exited without completing all menu checks (exit=$exit_code)." >&2
    exit 1
  fi
  if grep -q 'macos-startup-smoke-failed' "$report_dir/console.log"; then
    cat "$report_dir/console.log"
    exit 1
  fi
  sleep 1
done
cat "$report_dir/console.log"
echo 'SnapPin startup/menu quit smoke test timed out.' >&2
exit 1
