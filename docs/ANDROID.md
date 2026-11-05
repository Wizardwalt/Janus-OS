# JanusOS on Android

JanusOS ships as an Android 13 image for the Radxa CM5 (Pandora Titan handheld)
with JanusCore installed as a privileged platform app, and as a plain APK for any
Android 12+ device. This document is the build, provisioning and troubleshooting
manual for both.

- [What runs where](#what-runs-where)
- [Architecture](#architecture)
- [Building the APK](#building-the-apk)
- [Building the CM5 image](#building-the-cm5-image)
- [Provisioning (Device Owner / kiosk)](#provisioning-device-owner--kiosk)
- [Permissions model](#permissions-model)
- [Runtime contract (JNI)](#runtime-contract-jni)
- [Security posture](#security-posture)
- [Troubleshooting](#troubleshooting)

## What runs where

| Piece | Language | Lives in | Runs as |
| --- | --- | --- | --- |
| Engine (`janus-core`, `janus-runtime`) | Rust | `janus-core/`, `janus-runtime/` | in-process library inside the app |
| JNI bridge | Rust | `janus-android/` → `libjanus_android.so` | in-process |
| Kiosk shell, service, bridges | Kotlin | `app/src/main/java/com/janus/omega/` | app process |
| Dashboard | HTML/CSS/ES | `web/` → APK assets → `/data/data/com.janus.omega/files/engine/web` | WebView over loopback |
| Module catalogue | Lua | `plugins/`, `core/`, `modules/`, `apocalypse_engineering/` | executed by the engine |
| Image overlay | AOSP make/blueprint | `android/device/janus/titan/` | build system |

The desktop/server targets (`iso/`, `deploy/`, `janus-tui`) are unaffected: the
same Rust crates are used, `janus-runtime` is simply a library now instead of a
binary-only crate, and `janus-runtime --config …` still starts the server.

## Architecture

```
        ┌──────────────────────── app process (com.janus.omega) ───────────────────────┐
        │                                                                              │
        │  MainActivity (kiosk shell)                                                  │
        │    └─ WebView ── http://127.0.0.1:<port>/ ──┐                                │
        │       window.janusDevice ── DeviceBridge    │                                │
        │                                             ▼                                │
        │  JanusRuntimeService (foreground, specialUse)                                │
        │    └─ NativeBridge (JNI) ── libjanus_android.so ── janus_runtime ── SQLite   │
        │                                  ├─ axum API (loopback, ephemeral port)      │
        │                                  ├─ Lua executor (spawn_blocking pool)       │
        │                                  └─ ServeDir → extracted web/                │
        │                                                                              │
        │  BootReceiver / JanusDeviceAdminReceiver ── DeviceOwnerManager (kiosk)       │
        └──────────────────────────────────────────────────────────────────────────────┘
```

Boot order on the CM5 image:

1. `BOOT_COMPLETED` → `BootReceiver` starts `JanusRuntimeService` and, when the
   app is Device Owner, applies kiosk policy and launches `MainActivity`.
2. The service unpacks the asset payload on first run (or after an upgrade),
   then calls `nativeStart` and waits for the engine to report a port.
3. `MainActivity` binds to the service; when the state is `Running` it loads
   `http://127.0.0.1:<port>/` and applies lock task.

The port is ephemeral (`port = 0` in the config) and never leaves the process
except through the kiosk shell, so there is no fixed port to collide with and
nothing to firewall.

## Building the APK

Prerequisites:

```bash
rustup target add aarch64-linux-android     # and/or x86_64-linux-android
cargo install cargo-ndk
export ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/26.1.10909125
export ANDROID_SDK_ROOT=$HOME/Android/Sdk
gradle wrapper --gradle-version 8.5         # once, to create gradle-wrapper.jar
```

Then:

```bash
scripts/build-android.sh                    # arm64 debug APK
scripts/build-android.sh --release --full    # minified, with the full catalogue
scripts/build-android.sh --abi x86_64        # emulator
```

What the script does: `cargo ndk` builds `libjanus_android.so` for the ABI, copies
it into `app/build/janusJniLibs/<abi>/`, and runs `./gradlew assembleDebug`
with `-PjanusRustAbi` and `-PjanusFullCatalogue`. The Gradle `syncJanusAssets`
task copies `web/`, `plugins/`, `core/` (plus `modules/` and
`apocalypse_engineering/` when `--full`) into the APK assets, and the service
extracts them to `files/engine/` on first start.

Sizes: the lean APK is dominated by the Rust library (~8–12 MB); `--full` adds
about 38 MB of Lua.

Install:

```bash
adb install -r app/build/outputs/apk/release/app-release.apk
```

Emulator note: build the `x86_64` ABI; the default `arm64-v8a` build will install
but fail to load the library (`NativeBridge.load()` returns false and the overlay
reports `libjanus_android.so is not packaged for this ABI`).

## Building the CM5 image

The image is the Radxa CM5 AOSP tree plus the overlay in
`android/device/janus/titan/`. Nothing in the vendor tree is patched.

```bash
# 1. Radxa CM5 Android 13 SDK
repo init -u https://github.com/radxa/manifests -b <android-13-branch>
repo sync -j8

# 2. Add the JanusOS overlay
cp -a /path/to/Janus-OS/android/device/janus device/radxa-cm5-overlay
```

Place the overlay where the build system will see it — the simplest is inside
the Radxa tree:

```bash
cp -a /path/to/Janus-OS/android/device/janus device/janus
```

Register it in `build/make/target/product/AndroidProducts.mk` (or point
`JANUS_BASE_PRODUCT` at the real Radxa product makefile if the path differs):

```make
PRODUCT_MAKEFILES := \
    $(LOCAL_DIR)/../../device/janus/titan/janus_titan.mk
```

```bash
source build/envsetup.sh
lunch janus_titan-userdebug

# 3. Stage the application (built by scripts/build-android.sh --stage)
m JanusCore

# 4. Full image
. build.sh -UACKu
```

Images land in `rockdev/` (`boot.img`, `super.img`, `update.img`). Flash with the
Radxa `upgrade_tool` / `RKDevTool` as described in the vendor documentation.

The overlay does three things:

- `janus_titan.mk` inherits the Radxa CM5 product, adds `JanusCore` and the
  permission config modules, disables the setup wizard, and sets product identity.
- `janus_core/Android.bp` imports the Gradle-built APK as
  `android_app_import { certificate: "platform", privileged: true }`, which is
  what puts it in `/system/priv-app` and signs it with the platform key.
- `privapp-permissions-com.janus.omega.xml` and
  `default-permissions-com.janus.omega.xml` grant the signature and runtime
  permissions the appliance needs.

> The build re-signs the imported APK with the platform key. If you flash an
> image built with a different platform key than the one you installed
> JanusCore with, the app either fails to install (signature mismatch on
> upgrade) or loses its privileged permissions. Keep one keystore per device
> fleet.

## Provisioning (Device Owner / kiosk)

On the image, JanusCore still has to be promoted to Device Owner once; Android
does not allow an app to make itself Device Owner. Either:

- provision at first boot with a provisioning APK or `adb` (below), or
- add the app to the device's provisioning flow (`PROVISIONING_*` intents) if you
  build a factory-provisioned fleet.

No accounts may exist on the device when you provision:

```bash
adb shell pm list users            # must show only UserInfo{0:...}
adb shell dpm set-device-owner com.janus.omega/.device.JanusDeviceAdminReceiver
adb shell dpm list-owners           # verify
```

After that, the app applies kiosk policy on every start:

| Policy | Effect |
| --- | --- |
| `setLockTaskPackages([com.janus.omega])` | only this app may enter lock task |
| `setLockTaskFeatures(NONE)` | no home/recents/notifications in lock task |
| `setStatusBarDisabled(true)` | status bar pull-down disabled |
| `setKeyguardDisabled(true)` | no lock screen; boot goes straight to the dashboard |
| `setMaximumTimeToLock(0)` | screen never locks on its own |
| `addUserRestriction(SAFE_BOOT, ADD_USER, MOUNT_PHYSICAL_MEDIA, USB_FILE_TRANSFER, CONFIG_BLUETOOTH)` | no recovery escape hatches from the UI |

`DeviceOwnerManager.applyKioskPolicy` logs a warning for any policy the build
rejects and continues, so an older platform degrades instead of failing to boot.

Remove Device Owner for maintenance builds:

```bash
adb shell dpm remove-active-admin com.janus.omega/.device.JanusDeviceAdminReceiver
```

(Allowed here because the app does not hide the admin; on production fleets
require physical access before running this.)

## Permissions model

Declared in `app/src/main/AndroidManifest.xml`, granted as follows:

| Class | Permissions | Granted by |
| --- | --- | --- |
| Normal | `INTERNET`, `ACCESS_NETWORK_STATE`, `FOREGROUND_SERVICE`, `WAKE_LOCK`, `RECEIVE_BOOT_COMPLETED`, `QUERY_ALL_PACKAGES` | install |
| Runtime | `CAMERA`, `RECORD_AUDIO`, `ACCESS_FINE_LOCATION`, `READ_PHONE_STATE`, `POST_NOTIFICATIONS`, `BLUETOOTH_*` | image: `default-permissions-*.xml`; APK: user prompt (the app requests notifications itself, the rest on first use) |
| Signature \| privileged | `INSTALL_PACKAGES`, `DELETE_PACKAGES`, `WRITE_SECURE_SETTINGS`, `READ_LOGS`, `DUMP`, `REBOOT`, `MANAGE_USB`, `PACKAGE_USAGE_STATS` | image: `privapp-permissions-*.xml` + platform signature; APK: **not granted** |

The dashboard reports what it actually has: `window.janusDevice.permissions()`
returns the grant state of every runtime permission, and the HUD renders it next
to the battery and network panels. A sideloaded APK therefore shows real data for
device info, battery, network, installed packages and the engine, and degrades
predictably for telephony, secure settings and package installation.

## Runtime contract (JNI)

`com.janus.omega.core.NativeBridge` ↔ `janus-android/src/lib.rs`. Every call is
JSON in / JSON out; a Rust panic is caught by `catch_unwind` and returned as
`{"ok":false,...}` instead of aborting the process.

`nativeStart` receives:

```json
{
  "db_path": "/data/user/0/com.janus.omega/files/engine/janus.db",
  "plugin_dir": ".../files/engine/plugins",
  "core_dir": ".../files/engine/core",
  "web_dir": ".../files/engine/web",
  "bind": "127.0.0.1",
  "port": 0,
  "license_public_key": "<optional>",
  "audit_retention_days": 365
}
```

and answers:

```json
{"ok": true, "port": 41234, "version": "0.1.0"}
```

`nativeStop` shuts the engine down gracefully (up to 10 s, checkpointing SQLite),
`nativeStatus` reports whether a runtime is live. The engine owns its own
two-worker Tokio runtime, so the Android main thread never blocks.

The dashboard talks to exactly the same HTTP API as the desktop build: routes are
mounted both at `/` and `/api/`, so `web/runtime_api.js` works unchanged in the
WebView.

## Security posture

- **Loopback only.** The engine binds `127.0.0.1` on an ephemeral port; the
  manifest has `usesCleartextTraffic="false"` and the network security config
  permits cleartext for `127.0.0.1`/`localhost` only.
- **Navigation is pinned.** `shouldOverrideUrlLoading` rejects any URL whose host
  is not loopback, and file/content access is disabled in the WebView, so the
  JS bridge is only reachable from engine-served pages.
- **State-changing bridge calls are gated.** `janusDevice.reboot()` requires
  Device Owner; everything else in `DeviceBridge` is read-only device inventory.
- **Backup is off.** `allowBackup="false"` plus data-extraction rules exclude the
  database, credential hashes and licence material from cloud backup and
  device-to-device transfer.
- **Privileges are explicit.** The privileged permissions are listed in one file
  that ships with the image; a permission missing there is denied, which keeps
  the grant set auditable.
- **Release builds are minified**, with R8 rules that keep the JNI class, the
  `@JavascriptInterface` methods and the manifest components.

Known gaps (deliberate, tracked):

- The WebView bridge is available to any page the engine serves. If the engine is
  ever made to proxy arbitrary remote content, the bridge must move to
  `WebViewCompat.addWebMessageListener` with per-origin checks.
- Telephony, camera and sensor access is currently inventory/tuning via
  `DeviceBridge`; module-initiated capture (camera frames into a Lua module) is
  not wired yet.
- `janus-web` (the desktop demo server) is not used on Android; the Android build
  ships only the runtime.

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| Overlay: "libjanus_android.so is not packaged for this ABI" | ABI mismatch (arm64 APK on x86_64 emulator) | rebuild with `--abi x86_64` |
| Overlay: "engine library unavailable" on a real device | `jniLibs` was not staged | run `scripts/build-android.sh` (it copies the `.so`), not a bare `./gradlew` |
| Overlay: "Unpacking engine payload..." never finishes | asset tree is huge on slow eMMC | first run with `--full` takes a minute; subsequent boots reuse the stamp |
| Dashboard loads but modules list is empty | engine started before the catalogue was extracted | service extracts first, then starts; check `adb logcat -s JanusAssets JanusService` |
| `dpm set-device-owner` → "Not allowed to set the device owner" | a user or account already exists | factory reset, provision before adding accounts |
| Installed app has no privileged permissions | APK not in `/system/priv-app` or signed with another key | use `m JanusCore` inside the image build |
| Boot loops to launcher, not the dashboard | Device Owner not set | `adb shell dpm list-owners`; provision, then reboot |
| `adb logcat -s JanusNative JanusService JanusAssets JanusDeviceOwner` | — | start here for anything engine-related |
