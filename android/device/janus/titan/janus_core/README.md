# JanusCore (image import)

This directory is the hand-off between the Gradle build and the AOSP tree.

`Android.bp` imports `JanusCore.apk` from here as a privileged, platform-signed
application. The APK is **not** committed — it is staged by the build:

```bash
# from the JanusOS repository root
scripts/build-android.sh --release --full --stage
```

which copies `app/build/outputs/apk/release/app-release.apk` to
`JanusCore.apk` in this directory. Without that file, `m JanusCore` fails with a
missing-source error; that is intentional (a stale or absent APK should never be
silently packaged into an image).

The two XML files next to it are part of the image, not of the APK:

| File | Installed to | Purpose |
| --- | --- | --- |
| `privapp-permissions-com.janus.omega.xml` | `/system/etc/permissions/` | grants the signature\|privileged permissions declared in the app manifest |
| `default-permissions-com.janus.omega.xml` | `/system/etc/default-permissions/` | pre-grants camera/mic/location/telephony/notifications at install time |

Keep the first file in sync with the `tools:ignore="ProtectedPermissions"` block
in `app/src/main/AndroidManifest.xml`: declaring a privileged permission without
allowlisting it here results in a silent denial at runtime.
