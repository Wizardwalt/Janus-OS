package com.janus.omega

import android.os.Build
import android.os.Bundle
import android.os.StatFs
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import androidx.fragment.app.Fragment
import androidx.lifecycle.lifecycleScope
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class HardwareFragment : Fragment() {

    override fun onCreateView(
        inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?
    ): View = inflater.inflate(R.layout.fragment_hardware, container, false)

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)

        viewLifecycleOwner.lifecycleScope.launch {
            val values = withContext(Dispatchers.IO) {
                listOf(
                    R.id.tv_device to "  DEVICE:      ${Build.MANUFACTURER.uppercase()} ${Build.MODEL.uppercase()}",
                    R.id.tv_android to "  ANDROID:     ${Build.VERSION.RELEASE}  (API ${Build.VERSION.SDK_INT})",
                    R.id.tv_cpu to "  HARDWARE:    ${Build.HARDWARE.uppercase()}",
                    R.id.tv_arch to "  ARCH:        ${Build.SUPPORTED_ABIS.firstOrNull() ?: "UNKNOWN"}",
                    R.id.tv_board to "  BOARD:       ${Build.BOARD.uppercase()}",
                    R.id.tv_kernel to "  KERNEL:      ${System.getProperty("os.version") ?: "UNKNOWN"}",
                    R.id.tv_cores to "  CPU CORES:   ${Runtime.getRuntime().availableProcessors()}",
                    R.id.tv_ram to "  RAM:         ${getTotalRam()}",
                    R.id.tv_storage to "  STORAGE:     ${getStorageInfo()}",
                    R.id.tv_fingerprint to "  BUILD:       ${Build.FINGERPRINT.take(40)}"
                )
            }
            values.forEach { (id, text) ->
                view.findViewById<TextView>(id).text = text
            }
        }
    }

    private fun getTotalRam(): String {
        return try {
            val line = File("/proc/meminfo").bufferedReader().use { it.readLine() }
                ?: return "UNKNOWN"
            val kb = line.replace("MemTotal:", "").replace("kB", "").trim().toLong()
            "${kb / 1024} MB"
        } catch (_: Exception) {
            "UNKNOWN"
        }
    }

    private fun getStorageInfo(): String {
        return try {
            val stat = StatFs(android.os.Environment.getDataDirectory().path)
            val total = stat.blockCountLong * stat.blockSizeLong / (1024 * 1024 * 1024)
            val free = stat.availableBlocksLong * stat.blockSizeLong / (1024 * 1024 * 1024)
            "${free}GB FREE / ${total}GB TOTAL"
        } catch (_: Exception) {
            "UNKNOWN"
        }
    }
}
