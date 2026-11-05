package com.janus.omega.device

import android.app.admin.DeviceAdminReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * Device administration receiver declared in the manifest.
 *
 * Required for Device Owner status: the provisioning step promotes this
 * component, which is what allows the kiosk policies in [DeviceOwnerManager] to
 * be applied without user interaction.
 */
class JanusDeviceAdminReceiver : DeviceAdminReceiver() {

    override fun onEnabled(context: Context, intent: Intent) {
        Log.i(TAG, "device admin enabled")
    }

    override fun onDisabled(context: Context, intent: Intent) {
        Log.w(TAG, "device admin disabled; kiosk policy is no longer enforceable")
    }

    override fun onLockTaskModeEntering(context: Context, intent: Intent, pkg: String) {
        Log.i(TAG, "lock task entered by $pkg")
    }

    override fun onLockTaskModeExiting(context: Context, intent: Intent) {
        Log.w(TAG, "lock task exited")
    }

    private companion object {
        const val TAG = "JanusDeviceAdmin"
    }
}
