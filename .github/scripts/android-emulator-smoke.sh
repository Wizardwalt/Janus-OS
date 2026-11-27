#!/usr/bin/env bash
set -euo pipefail

PACKAGE="com.janus.omega"
mkdir -p test-results

# The emulator action can report a connected device before Android finishes booting.
timeout 900 adb wait-for-device
boot_complete=0
for attempt in $(seq 1 120); do
  boot_status="$(timeout 10 adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r' || true)"
  if [[ "$boot_status" == "1" ]]; then
    boot_complete=1
    break
  fi
  sleep 2
done
test "$boot_complete" = "1"
timeout 15 adb devices -l > test-results/adb-devices.txt 2>&1 || true
timeout 15 adb shell getprop sys.boot_completed > test-results/boot-completed.txt 2>&1 || true

# sys.boot_completed can precede SystemUI readiness by several minutes on TCG.
# Do not launch Janus while the home window manager still has no focused window.
home_ready=0
for attempt in $(seq 1 60); do
  timeout 5 adb shell dumpsys window windows > test-results/boot-windows.txt 2>&1 || true
  if grep -E 'mCurrentFocus=Window\\{|imeLayeringTarget in display#[0-9]+ Window\\{' test-results/boot-windows.txt | grep -Fq 'com.android.launcher3'; then
    home_ready=1
    break
  fi
  sleep 5
done
if [[ "$home_ready" != "1" ]]; then
  timeout 30 adb logcat -d -t 10000 > test-results/boot-logcat.txt 2>&1 || true
  echo "Android System UI did not reach a responsive launcher window after boot." >&2
  grep -E 'mCurrentFocus=|mFocusedApp=|imeLayeringTarget|System UI isn.t responding' test-results/boot-windows.txt | tail -n 40 >&2 || true
  exit 1
fi

timeout 60 adb install -r app/build/outputs/apk/debug/app-debug.apk > test-results/install.txt 2>&1
timeout 30 adb shell am force-stop "$PACKAGE"
timeout 60 adb shell am start -W -n "$PACKAGE/.MainActivity" > test-results/launch.txt 2>&1

pid=""
for attempt in $(seq 1 60); do
  pid="$(timeout 15 adb shell pidof "$PACKAGE" 2>/dev/null | tr -d '\r' || true)"
  if [[ -n "$pid" ]]; then
    break
  fi
  sleep 1
done

# Collect evidence before any assertion can terminate the job. In particular,
# a crash immediately after launch must still leave logcat and activity state.
printf '%s\n' "$pid" > test-results/pidof.txt
timeout 30 adb shell dumpsys activity activities > test-results/activities.txt 2>&1 || true
timeout 30 adb logcat -d -t 10000 > test-results/emulator-logcat.txt 2>&1 || true

# A live PID and a resumed Activity can still be the Android starting splash
# window. Require the actual Janus view hierarchy to be drawn before passing.
ui_ready=0
for attempt in $(seq 1 12); do
  timeout 12 adb shell uiautomator dump --compressed /sdcard/janus-window.xml \
    > test-results/uiautomator-dump.txt 2>&1 || true
  timeout 8 adb shell cat /sdcard/janus-window.xml \
    > test-results/janus-window.xml 2>/dev/null || true
  if grep -q 'JANUS OMEGA' test-results/janus-window.xml &&
     grep -q 'DASHBOARD' test-results/janus-window.xml &&
     grep -q 'CATALOG' test-results/janus-window.xml &&
     grep -q 'HARDWARE' test-results/janus-window.xml &&
     grep -q '0 EXECUTABLE' test-results/janus-window.xml &&
     grep -q '260 CATALOGED' test-results/janus-window.xml; then
    ui_ready=1
    break
  fi
  sleep 2
done

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

if [[ "$ui_ready" != "1" ]]; then
  echo "Janus process and Activity started, but the expected Janus UI did not render." >&2
  cat test-results/uiautomator-dump.txt >&2 || true
  grep -E 'mResumedActivity|topResumedActivity|ResumedActivity:|Resumed:' test-results/activities.txt | tail -n 40 >&2 || true
  exit 1
fi

if grep -q 'FATAL EXCEPTION' test-results/emulator-logcat.txt; then
  echo 'Fatal Android exception found in emulator log.' >&2
  exit 1
fi
