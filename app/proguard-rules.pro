# R8 keeps what JNI and the WebView reflectively reach into.

# JNI: symbol names encode the fully-qualified class and method names, so the
# class must not be renamed or its native methods stripped.
-keep class com.janus.omega.core.NativeBridge { *; }
-keepclasseswithmembernames,includedescriptorclasses class * {
    native <methods>;
}

# WebView bridge: @JavascriptInterface methods are looked up by name from JS.
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}
-keep class com.janus.omega.device.DeviceBridge { *; }

# Android components referenced only from the manifest.
-keep class com.janus.omega.MainActivity { *; }
-keep class com.janus.omega.JanusRuntimeService { *; }
-keep class com.janus.omega.device.JanusDeviceAdminReceiver { *; }
-keep class com.janus.omega.device.BootReceiver { *; }

# org.json is part of the platform; silence R8's optional lookups.
-dontwarn org.json.**
