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

adb install -r app/build/outputs/apk/debug/app-debug.apk > test-results/install.txt 2>&1
adb shell am force-stop "$PACKAGE"
adb shell monkey -p "$PACKAGE" -c android.intent.category.LAUNCHER 1 > test-results/monkey-launch.txt 2>&1 || true

pid=""
for attempt in $(seq 1 60); do
  pid="$(adb shell pidof "$PACKAGE" 2>/dev/null | tr -d '\r' || true)"
  if [[ -n "$pid" ]]; then
    break
  fi
  sleep 1
done

# Collect evidence before any assertion can terminate the job. In particular,
# a crash immediately after launch must still leave logcat and activity state.
printf '%s\n' "$pid" > test-results/pidof.txt
adb shell dumpsys activity activities > test-results/activities.txt 2>&1 || true
adb logcat -d -t 10000 > test-results/emulator-logcat.txt 2>&1 || true

if [[ -z "$pid" ]]; then
  echo "Janus process did not remain alive after launcher invocation." >&2
  grep -E 'FATAL EXCEPTION|AndroidRuntime|Process:|Unable to start' test-results/emulator-logcat.txt | tail -n 80 >&2 || true
  exit 1
fi

if ! grep -E 'mResumedActivity|topResumedActivity|ResumedActivity:|Resumed:' test-results/activities.txt | grep -q "$PACKAGE"; then
  echo "Janus process is alive but no resumed Janus activity was found." >&2
  grep -E 'mResumedActivity|topResumedActivity|ResumedActivity:|Resumed:' test-results/activities.txt | tail -n 40 >&2 || true
  exit 1
fi

if grep -q 'FATAL EXCEPTION' test-results/emulator-logcat.txt; then
  echo 'Fatal Android exception found in emulator log.' >&2
  exit 1
fi
