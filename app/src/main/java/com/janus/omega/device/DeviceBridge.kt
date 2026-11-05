package com.janus.omega.device

import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.BatteryManager
import android.os.Build
import android.telephony.TelephonyManager
import android.util.Log
import android.webkit.JavascriptInterface
import com.janus.omega.BuildConfig
import org.json.JSONArray
import org.json.JSONObject

/**
 * The dashboard's window onto the device.
 *
 * Exposed to the kiosk WebView as `window.janusDevice`. Every method returns a
 * JSON string and never throws across the bridge: a `SecurityException` from a
 * permission the build did not grant comes back as `{"ok":false,"error":...}`,
 * which the dashboard renders instead of leaving a blank panel.
 *
 * This is deliberately read-mostly. Anything that changes device state (reboot,
 * lock task) is gated on Device Owner so the bridge cannot be used to escalate
 * from a web page that somehow reached the WebView.
 */
class DeviceBridge(private val context: Context) {

    @Volatile
    private var enginePort: Int = 0

    fun updatePort(port: Int) {
        enginePort = port
    }

    /** Identity, build info and the handful of states the HUD always shows. */
    @JavascriptInterface
    fun info(): String = envelope {
        val battery = batteryJson()
        val network = networkJson()
        put("package", context.packageName)
        put("versionName", BuildConfig.VERSION_NAME)
        put("versionCode", BuildConfig.VERSION_CODE)
        put("fullCatalogue", BuildConfig.FULL_CATALOGUE)
        put("enginePort", enginePort)
        put("deviceOwner", DeviceOwnerManager.isDeviceOwner(context))
        put("lockTask", DeviceOwnerManager.isLockTaskActive(context))
        put("manufacturer", Build.MANUFACTURER)
        put("model", Build.MODEL)
        put("device", Build.DEVICE)
        put("androidRelease", Build.VERSION.RELEASE)
        put("sdkInt", Build.VERSION.SDK_INT)
        put("battery", battery)
        put("network", network)
    }

    @JavascriptInterface
    fun battery(): String = envelope { put("battery", batteryJson()) }

    @JavascriptInterface
    fun network(): String = envelope { put("network", networkJson()) }

    @JavascriptInterface
    fun telephony(): String = envelope { put("telephony", telephonyJson()) }

    /**
     * Installed packages. System packages are included only on request because
     * the list is long on a stock CM5 image.
     */
    @JavascriptInterface
    fun packages(includeSystem: Boolean): String = envelope {
        val manager = context.packageManager
        val installed = manager.getInstalledPackages(0)
        val entries = JSONArray()
        installed.forEach { info ->
            val isSystem = (info.applicationInfo?.flags ?: 0) and
                android.content.pm.ApplicationInfo.FLAG_SYSTEM != 0
            if (isSystem && !includeSystem) return@forEach
            entries.put(
                JSONObject().apply {
                    put("package", info.packageName)
                    put("label", info.applicationInfo?.let { manager.getApplicationLabel(it).toString() } ?: info.packageName)
                    put("versionName", info.versionName ?: "")
                    put("versionCode", if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) info.longVersionCode else info.versionCode.toLong())
                    put("system", isSystem)
                },
            )
        }
        put("packages", entries)
        put("count", entries.length())
    }

    /**
     * Grant state for the permissions the engine's Android bridges rely on.
     * Surfacing it here makes a mis-provisioned build obvious in the HUD rather
     * than as a silently empty telephony panel.
     */
    @JavascriptInterface
    fun permissions(): String = envelope {
        val wanted = listOf(
            android.Manifest.permission.READ_PHONE_STATE,
            android.Manifest.permission.ACCESS_FINE_LOCATION,
            android.Manifest.permission.CAMERA,
            android.Manifest.permission.RECORD_AUDIO,
            android.Manifest.permission.READ_EXTERNAL_STORAGE,
            android.Manifest.permission.POST_NOTIFICATIONS,
        )
        val result = JSONObject()
        wanted.forEach { permission ->
            val granted = context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
            result.put(permission.substringAfterLast('.'), granted)
        }
        put("permissions", result)
    }

    /** Device Owner only; returns an error rather than rebooting on request from a page. */
    @JavascriptInterface
    fun reboot(): String = envelope {
        if (!DeviceOwnerManager.reboot(context)) {
            throw SecurityException("reboot requires Device Owner and REBOOT permission")
        }
        put("rebooting", true)
    }

    private fun batteryJson(): JSONObject {
        val manager = context.getSystemService(Context.BATTERY_SERVICE) as? BatteryManager
        val sticky = context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val level = sticky?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
        val scale = sticky?.getIntExtra(BatteryManager.EXTRA_SCALE, -1) ?: -1
        val percent = when {
            level >= 0 && scale > 0 -> (level * 100) / scale
            manager != null -> manager.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY)
            else -> -1
        }
        val status = sticky?.getIntExtra(BatteryManager.EXTRA_STATUS, -1) ?: -1
        return JSONObject().apply {
            put("percent", percent)
            put("charging", status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL)
            put("status", status)
        }
    }

    private fun networkJson(): JSONObject {
        val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            ?: return JSONObject().put("available", false)
        val active = manager.activeNetwork
        if (active == null) return JSONObject().put("available", false)
        val caps = manager.getNetworkCapabilities(active)
        val transports = JSONArray()
        if (caps != null) {
            if (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) transports.put("wifi")
            if (caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) transports.put("cellular")
            if (caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) transports.put("ethernet")
            if (caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) transports.put("vpn")
            if (caps.hasTransport(NetworkCapabilities.TRANSPORT_BLUETOOTH)) transports.put("bluetooth")
        }
        return JSONObject().apply {
            put("available", true)
            put("transports", transports)
            put("metered", manager.isActiveNetworkMetered)
            put("validated", caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED) ?: false)
        }
    }

    private fun telephonyJson(): JSONObject {
        val manager = context.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager
            ?: return JSONObject().put("available", false)
        return try {
            JSONObject().apply {
                put("available", true)
                put("operator", manager.networkOperatorName ?: "")
                put("simOperator", manager.simOperatorName ?: "")
                put("networkType", manager.dataNetworkType)
                put("simState", manager.simState)
                put("roaming", manager.isNetworkRoaming)
                put("phoneType", manager.phoneType)
            }
        } catch (error: SecurityException) {
            Log.w(TAG, "telephony denied", error)
            JSONObject().apply {
                put("available", false)
                put("error", "READ_PHONE_STATE not granted")
            }
        }
    }

    private inline fun envelope(body: JSONObject.() -> Unit): String = try {
        JSONObject().apply {
            put("ok", true)
            body()
        }.toString()
    } catch (error: Throwable) {
        Log.w(TAG, "bridge call failed", error)
        JSONObject().apply {
            put("ok", false)
            put("error", error.message ?: error.javaClass.simpleName)
        }.toString()
    }

    private companion object {
        const val TAG = "JanusDeviceBridge"
    }
}
