#!/usr/bin/env bash
set -euo pipefail

PACKAGE="com.janus.omega"
mkdir -p test-results

# The emulator action can report a connected device before Android finishes booting.
timeout 900 adb wait-for-device
boot_complete=0
for attempt in $(seq 1 120); do
  if [[ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]]; then
    boot_complete=1
    break
  fi
  sleep 2
done
test "$boot_complete" = "1"
adb devices -l > test-results/adb-devices.txt
adb shell getprop sys.boot_completed > test-results/boot-completed.txt

adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am force-stop "$PACKAGE"
adb shell monkey -p "$PACKAGE" -c android.intent.category.LAUNCHER 1

pid=""
for attempt in $(seq 1 60); do
  pid="$(adb shell pidof "$PACKAGE" 2>/dev/null | tr -d '\r' || true)"
  if [[ -n "$pid" ]]; then
    break
  fi
  sleep 1
done
test -n "$pid"

adb shell dumpsys activity activities > test-results/activities.txt
grep -E 'mResumedActivity|topResumedActivity|ResumedActivity:|Resumed:' test-results/activities.txt | grep "$PACKAGE"
adb logcat -d -t 5000 > test-results/emulator-logcat.txt
if grep -q 'FATAL EXCEPTION' test-results/emulator-logcat.txt; then
  echo 'Fatal Android exception found in emulator log.' >&2
  exit 1
fi
