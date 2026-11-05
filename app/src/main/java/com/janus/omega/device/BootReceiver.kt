package com.janus.omega.device

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import com.janus.omega.JanusRuntimeService
import com.janus.omega.MainActivity

/**
 * Brings the appliance back up without a human touching it.
 *
 * Two things have to happen after a cold boot: the engine service starts, and —
 * when this app is the Device Owner — the kiosk activity is launched. Only a
 * device owner (or a persistent system app) may start an activity from the
 * background, which is exactly why the CM5 image provisions this app as Device
 * Owner. On a plain APK install the receiver still starts the service and the
 * user opens the app normally.
 */
class BootReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED,
            Intent.ACTION_MY_PACKAGE_REPLACED,
            -> {
                Log.i(TAG, "boot signal: ${intent.action}")
                JanusRuntimeService.start(context)
                if (DeviceOwnerManager.isDeviceOwner(context)) {
                    DeviceOwnerManager.applyKioskPolicy(context)
                    launchKiosk(context)
                }
            }

            else -> Unit
        }
    }

    private fun launchKiosk(context: Context) {
        runCatching {
            context.startActivity(
                Intent(context, MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP),
            )
        }.onFailure {
            Log.w(TAG, "cannot launch kiosk from boot; waiting for user", it)
        }
    }

    private companion object {
        const val TAG = "JanusBoot"
    }
}
