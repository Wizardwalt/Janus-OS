package com.janus.omega

import android.content.Context
import android.net.wifi.WifiManager
import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import androidx.fragment.app.Fragment
import androidx.lifecycle.lifecycleScope
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class DashboardFragment : Fragment() {

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?
    ): View = inflater.inflate(R.layout.fragment_dashboard, container, false)

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)

        val sdf = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault())
        view.findViewById<TextView>(R.id.tv_time).text = "  TIME:    ${sdf.format(Date())}"

        val rootView = view.findViewById<TextView>(R.id.tv_root)
        val wifiView = view.findViewById<TextView>(R.id.tv_wifi)
        rootView.text = "  ROOT CHECK: CHECKING COMMON PATHS..."
        wifiView.text = "  WI-FI:   CHECKING…"

        // Filesystem probes and Wi-Fi service Binder calls can block on slow or
        // partially booted devices. Keep them off the main thread so the first
        // dashboard frame is not held behind system-service initialization.
        val appContext = requireContext().applicationContext
        viewLifecycleOwner.lifecycleScope.launch {
            val (rooted, wifiState) = withContext(Dispatchers.IO) {
                val hasCommonSuPath = isDeviceRooted()
                val networkState = runCatching {
                    val wifiManager = appContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
                    val wifiInfo = wifiManager.connectionInfo
                    val ssid = wifiInfo?.ssid?.replace("\"", "") ?: "UNKNOWN"
                    if (wifiManager.isWifiEnabled) "ACTIVE :: $ssid" else "OFFLINE"
                }.getOrElse { "UNAVAILABLE" }
                hasCommonSuPath to networkState
            }

            rootView.text = if (rooted) {
                "  ROOT CHECK: COMMON su PATH DETECTED"
            } else {
                "  ROOT CHECK: NO COMMON su PATH FOUND"
            }
            rootView.setTextColor(if (rooted) 0xFFFFB020.toInt() else 0xFFB0B0B0.toInt())
            wifiView.text = "  WI-FI:   $wifiState"
        }

        view.findViewById<TextView>(R.id.tv_modules).text =
            "  MODULES: 0 EXECUTABLE / ${ModuleData.totalEntries} CATALOGED"
        view.findViewById<TextView>(R.id.tv_catalog_total).text =
            "  CATALOG ENTRIES: ${ModuleData.totalEntries}"
    }

    private fun isDeviceRooted(): Boolean {
        val paths = arrayOf(
            "/system/bin/su", "/system/xbin/su", "/sbin/su",
            "/data/local/xbin/su", "/data/local/bin/su", "/system/sd/xbin/su"
        )
        return paths.any { java.io.File(it).exists() }
    }
}
