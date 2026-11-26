package com.janus.omega

import android.net.wifi.WifiManager
import android.os.Bundle
import android.content.Context
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import androidx.fragment.app.Fragment
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class DashboardFragment : Fragment() {

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?
    ): View = inflater.inflate(R.layout.fragment_dashboard, container, false)

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)

        val sdf = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault())
        view.findViewById<TextView>(R.id.tv_time).text = "  TIME:    ${sdf.format(Date())}"

        val rooted = isDeviceRooted()
        view.findViewById<TextView>(R.id.tv_root).apply {
            text = if (rooted) "  ROOT CHECK: COMMON su PATH DETECTED" else "  ROOT CHECK: NO COMMON su PATH FOUND"
            setTextColor(if (rooted) 0xFF00FF41.toInt() else 0xFFFF4444.toInt())
        }

        val wifiMgr = requireContext().applicationContext
            .getSystemService(Context.WIFI_SERVICE) as WifiManager
        val wifiInfo = wifiMgr.connectionInfo
        val ssid = wifiInfo?.ssid?.replace("\"", "") ?: "NONE"
        val wifiState = if (wifiMgr.isWifiEnabled) "ACTIVE :: $ssid" else "OFFLINE"
        view.findViewById<TextView>(R.id.tv_wifi).text = "  WI-FI:   $wifiState"

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
