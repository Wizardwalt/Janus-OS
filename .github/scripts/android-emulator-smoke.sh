#!/usr/bin/env bash
set -euo pipefail

mkdir -p test-results
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am force-stop com.janus.omega
adb shell monkey -p com.janus.omega -c android.intent.category.LAUNCHER 1

for attempt in $(seq 1 30); do
  if adb shell pidof com.janus.omega >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

pid="$(adb shell pidof com.janus.omega | tr -d '\r')"
test -n "$pid"
adb shell dumpsys activity activities > test-results/activities.txt
grep -E 'mResumedActivity|topResumedActivity' test-results/activities.txt | grep com.janus.omega
adb logcat -d -t 5000 > test-results/emulator-logcat.txt
if grep -q 'FATAL EXCEPTION' test-results/emulator-logcat.txt; then
  echo 'Fatal Android exception found in emulator log.' >&2
  exit 1
fi
