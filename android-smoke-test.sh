#!/usr/bin/env bash
# Tries Toki Hub for Android on a running emulator or phone (adb): installs and opens it, and checks
# it shows its library. With INSTALL_GAME=<title> it then installs that game's newest Android
# version through the hub (and Android's installer, as a player would), and plays it.
# Screenshots and the screen's contents go to the folder given.
#
#   ./android-smoke-test.sh dist/Toki-Hub-android.apk [out folder]
set -euo pipefail

APK="$1"
OUT="${2:-target/android-test}"
HUB=com.rowstar10.tokihub
mkdir -p "$OUT"
step=0

shot() { step=$((step + 1)); adb exec-out screencap -p > "$OUT/$step-$1.png"; echo "screenshot: $step-$1"; }
fail() { shot failed || true; adb logcat -d > "$OUT/logcat.txt" || true; echo "FAILED: $*"; exit 1; }

# The screen's contents (Android's own views, and the hub's page inside its web view)
screen() { adb shell uiautomator dump /sdcard/screen.xml > /dev/null 2>&1 && adb exec-out cat /sdcard/screen.xml; }

# Where a control with this text (or description) is, in an app whose package matches $1 (a
# regular expression): "x y", or nothing
find() {
  screen | python3 -c '
import re, sys
app, want = sys.argv[1], [w.lower() for w in sys.argv[2:]]
for node in re.findall(r"<node [^>]*>", sys.stdin.read()):
    label = " ".join(re.findall(r"(?:text|content-desc)=\"([^\"]*)\"", node)).strip().lower()
    package = (re.search(r"package=\"([^\"]*)\"", node) or [None, ""])[1]
    if label in want and re.search(app, package):
        x1, y1, x2, y2 = map(int, re.search(r"bounds=\"\[(\d+),(\d+)\]\[(\d+),(\d+)\]\"", node).groups())
        if x2 > x1 and y2 > y1:
            print((x1 + x2) // 2, (y1 + y2) // 2)
            break
' "$@"
}

# Wait up to $1 seconds for a control in an app matching $2, then tap it
tap() {
  local seconds="$1"; shift
  for _ in $(seq "$seconds"); do
    local at; at="$(find "$@" || true)"
    if [[ -n "$at" ]]; then adb shell input tap $at; return 0; fi
    sleep 1
  done
  return 1
}

waitfor() {   # waitfor <seconds> <command...>: until the command succeeds
  local seconds="$1"; shift
  for _ in $(seq "$seconds"); do "$@" > /dev/null 2>&1 && return 0; sleep 1; done
  return 1
}

shows() { screen | grep -qi "$1"; }

echo "Installing $APK"
adb install -r "$APK"
# What a player allows the first time Toki Hub installs a game
adb shell appops set "$HUB" REQUEST_INSTALL_PACKAGES allow || true
adb logcat -c
adb shell am start -n "$HUB/.MainActivity"
waitfor 60 adb shell pidof "$HUB" || fail "Toki Hub didn't start"
waitfor 90 shows "Library\|Hu Taong\|No games yet" || fail "Toki Hub's library never showed"
sleep 3
shot hub
screen > "$OUT/hub-screen.xml"

if [[ -n "${INSTALL_GAME:-}" ]]; then
  package="${GAME_PACKAGE:?set GAME_PACKAGE to the Android package of the game}"
  adb shell pm uninstall "$package" > /dev/null 2>&1 || true
  tap 20 "$HUB" "$INSTALL_GAME" || true               # its card in the library
  sleep 2
  tap 30 "$HUB" "Install" || fail "no Install button for $INSTALL_GAME"
  shot downloading
  # Android's installer asks once the download is done (it can take a few minutes)
  tap 600 "packageinstaller" "Install" "Update" || fail "Android's installer never asked"
  waitfor 180 sh -c "adb shell pm list packages $package | grep -q $package" || fail "$INSTALL_GAME didn't install"
  shot installed
  adb shell am start -n "$HUB/.MainActivity" > /dev/null
  tap 60 "$HUB" "Play" || fail "no Play button once $INSTALL_GAME was installed"
  waitfor 60 adb shell pidof "$package" || fail "$INSTALL_GAME didn't start"
  sleep 20                                             # it unpacks itself the first time
  shot playing
fi

adb shell pidof "$HUB" > /dev/null || fail "Toki Hub closed by itself"
if adb logcat -d | grep -q "FATAL EXCEPTION\|panicked at"; then fail "something crashed: see logcat.txt"; fi
adb logcat -d > "$OUT/logcat.txt"
echo "Smoke test passed"
