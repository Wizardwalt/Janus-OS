package com.janus.omega.device

import android.app.Activity
import android.app.ActivityManager
import android.app.admin.DevicePolicyManager
import android.content.ComponentName
import android.content.Context
import android.os.Build
import android.os.UserManager
import android.util.Log

/**
 * Wraps the Device Owner APIs the kiosk depends on.
 *
 * Everything here is a no-op unless this app is the provisioned Device Owner: on
 * the CM5 image that is guaranteed by the provisioning step in
 * `docs/ANDROID.md`, and on a plain APK install the app simply runs as a normal
 * launcher, which keeps the same binary usable on both targets.
 */
object DeviceOwnerManager {

    private const val TAG = "JanusDeviceOwner"

    private fun policyManager(context: Context): DevicePolicyManager? =
        context.getSystemService(Context.DEVICE_POLICY_SERVICE) as? DevicePolicyManager

    private fun adminComponent(context: Context): ComponentName =
        ComponentName(context, JanusDeviceAdminReceiver::class.java)

    fun isDeviceOwner(context: Context): Boolean = try {
        policyManager(context)?.isDeviceOwnerApp(context.packageName) == true
    } catch (error: Throwable) {
        Log.w(TAG, "cannot query device owner state", error)
        false
    }

    /**
     * Applies the single-app policy. Returns whether this app owns the device,
     * so callers can decide between lock task and a normal launcher experience.
     */
    fun applyKioskPolicy(context: Context): Boolean {
        val dpm = policyManager(context) ?: return false
        if (!isDeviceOwner(context)) return false

        val admin = adminComponent(context)
        val restrictions = listOf(
            UserManager.DISALLOW_SAFE_BOOT,
            UserManager.DISALLOW_ADD_USER,
            UserManager.DISALLOW_MOUNT_PHYSICAL_MEDIA,
            UserManager.DISALLOW_USB_FILE_TRANSFER,
            UserManager.DISALLOW_CONFIG_BLUETOOTH,
        )

        // Each call can be rejected on older builds; a partially applied policy is
        // still better than refusing to start, so collect and continue.
        runCatching {
            dpm.setLockTaskPackages(admin, arrayOf(context.packageName))
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                dpm.setLockTaskFeatures(admin, DevicePolicyManager.LOCK_TASK_FEATURE_NONE)
            }
            dpm.setStatusBarDisabled(admin, true)
            dpm.setKeyguardDisabled(admin, true)
            dpm.setMaximumTimeToLock(admin, 0)
            restrictions.forEach { restriction ->
                runCatching { dpm.addUserRestriction(admin, restriction) }
                    .onFailure { Log.w(TAG, "restriction $restriction rejected", it) }
            }
        }.onFailure { Log.w(TAG, "kiosk policy partially applied", it) }

        return true
    }

    /** Pins the activity to the screen when lock task is available. */
    fun startLockTask(activity: Activity): Boolean {
        val dpm = policyManager(activity) ?: return false
        if (!isDeviceOwner(activity)) return false
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            if (dpm.getLockTaskPackages(adminComponent(activity)).toList().isEmpty()) {
                dpm.setLockTaskPackages(adminComponent(activity), arrayOf(activity.packageName))
            }
        }
        return runCatching {
            activity.startLockTask()
        }.onFailure {
            Log.w(TAG, "startLockTask failed", it)
        }.isSuccess
    }

    fun isLockTaskActive(context: Context): Boolean {
        val manager = context.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager ?: return false
        return manager.lockTaskModeState != ActivityManager.LOCK_TASK_MODE_NONE
    }

    /** Device owner is the only identity allowed to reboot the appliance. */
    fun reboot(context: Context): Boolean {
        val dpm = policyManager(context) ?: return false
        if (!isDeviceOwner(context)) return false
        return runCatching {
            dpm.reboot(adminComponent(context))
        }.onFailure {
            Log.w(TAG, "reboot rejected", it)
        }.isSuccess
    }
}
