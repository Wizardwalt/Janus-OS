package com.janus.omega

import android.annotation.SuppressLint
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.util.Log
import android.view.View
import android.view.WindowManager
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import com.janus.omega.device.DeviceBridge
import com.janus.omega.device.DeviceOwnerManager
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The kiosk shell.
 *
 * It hosts a fullscreen WebView pointed at the dashboard the Rust engine serves
 * on loopback, plus a status overlay that covers the "engine is still starting"
 * and "engine died" states. Nothing about the UI is reimplemented in Kotlin: the
 * dashboard in `web/` is the interface.
 *
 * Boot sequence: bind to [JanusRuntimeService], wait for [JanusRuntimeService.State.Running],
 * then load `http://127.0.0.1:<port>/`.
 */
class MainActivity : AppCompatActivity(), JanusRuntimeService.Listener {

    private lateinit var webView: WebView
    private lateinit var overlay: View
    private lateinit var overlayTitle: TextView
    private lateinit var overlayMessage: TextView
    private lateinit var retryButton: Button
    private lateinit var deviceBridge: DeviceBridge

    private var service: JanusRuntimeService? = null
    private var bound = false
    private var loadedPort = 0
    private val overlayVisible = AtomicBoolean(true)

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
            val local = (binder as? JanusRuntimeService.LocalBinder)?.service()
            if (local == null) {
                Log.e(TAG, "unexpected binder from runtime service")
                return
            }
            service = local
            bound = true
            local.addListener(this@MainActivity)
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            service?.removeListener(this@MainActivity)
            service = null
            bound = false
            onState(JanusRuntimeService.State.Stopped)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        webView = findViewById(R.id.kiosk_web_view)
        overlay = findViewById(R.id.overlay)
        overlayTitle = findViewById(R.id.overlay_title)
        overlayMessage = findViewById(R.id.overlay_message)
        retryButton = findViewById(R.id.overlay_retry)

        deviceBridge = DeviceBridge(this)
        configureWebView()

        retryButton.setOnClickListener {
            showOverlay("RESTARTING", "Reinitialising the engine...", showRetry = false)
            loadedPort = 0
            service?.restart() ?: startEngine()
        }

        applyKioskMode()
        requestNotificationPermission()
        startEngine()
        showOverlay("STARTING", "Bringing the Janus engine online...", showRetry = false)
    }

    override fun onStart() {
        super.onStart()
        bindService(
            Intent(this, JanusRuntimeService::class.java),
            connection,
            Context.BIND_AUTO_CREATE,
        )
    }

    override fun onStop() {
        if (bound) {
            service?.removeListener(this)
            runCatching { unbindService(connection) }
            bound = false
        }
        super.onStop()
    }

    override fun onResume() {
        super.onResume()
        webView.onResume()
        enterImmersive()
        // Re-assert lock task in case the system dropped it while backgrounded.
        if (DeviceOwnerManager.isDeviceOwner(this) && !DeviceOwnerManager.isLockTaskActive(this)) {
            DeviceOwnerManager.startLockTask(this)
        }
    }

    override fun onPause() {
        webView.onPause()
        super.onPause()
    }

    override fun onDestroy() {
        runCatching { webView.destroy() }
        super.onDestroy()
    }

    /** Kiosk rule: back never leaves the dashboard unless the device is unlocked. */
    @Deprecated("Overridden to keep the appliance in kiosk mode")
    override fun onBackPressed() {
        if (DeviceOwnerManager.isDeviceOwner(this)) return
        @Suppress("DEPRECATION")
        super.onBackPressed()
    }

    override fun onState(state: JanusRuntimeService.State) {
        runOnUiThread { render(state) }
    }

    private fun render(state: JanusRuntimeService.State) {
        when (state) {
            is JanusRuntimeService.State.Running -> {
                deviceBridge.updatePort(state.port)
                loadDashboard(state.port)
            }

            is JanusRuntimeService.State.Starting ->
                showOverlay("STARTING", "Bringing the Janus engine online...", showRetry = false)

            is JanusRuntimeService.State.Failed ->
                showOverlay("ENGINE OFFLINE", state.message, showRetry = true)

            JanusRuntimeService.State.Stopped ->
                showOverlay("ENGINE STOPPED", "The runtime is not running.", showRetry = true)
        }
    }

    private fun loadDashboard(port: Int) {
        if (port <= 0) return
        val url = "http://127.0.0.1:$port/"
        if (loadedPort == port && webView.url?.startsWith(url) == true && !overlayVisible.get()) return
        loadedPort = port
        hideOverlay()
        Log.i(TAG, "loading dashboard from $url")
        webView.loadUrl(url)
    }

    private fun startEngine() {
        JanusRuntimeService.start(this)
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun configureWebView() {
        webView.setBackgroundColor(Color.BLACK)
        webView.isVerticalScrollBarEnabled = false
        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            databaseEnabled = false
            // The dashboard is served by the engine itself; giving the WebView
            // file:// and content:// reach would only widen the attack surface.
            allowFileAccess = false
            allowContentAccess = false
            setSupportZoom(false)
            mediaPlaybackRequiresUserGesture = false
            mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            cacheMode = WebSettings.LOAD_DEFAULT
        }
        WebView.setWebContentsDebuggingEnabled(BuildConfig.DEBUG)

        webView.addJavascriptInterface(deviceBridge, "janusDevice")

        webView.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
                !isLoopback(request.url.host)

            override fun onReceivedError(
                view: WebView,
                request: WebResourceRequest,
                error: WebResourceError,
            ) {
                if (!request.isForMainFrame) return
                val description = error.description?.toString() ?: "connection failed"
                runOnUiThread { showOverlay("CONNECTION LOST", description, showRetry = true) }
            }
        }
    }

    private fun isLoopback(host: String?): Boolean =
        host == "127.0.0.1" || host == "localhost" || host == "::1"

    private fun applyKioskMode() {
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        if (DeviceOwnerManager.applyKioskPolicy(this)) {
            DeviceOwnerManager.startLockTask(this)
        }
    }

    private fun enterImmersive() {
        WindowCompat.setDecorFitsSystemWindows(window, false)
        val controller = WindowInsetsControllerCompat(window, window.decorView)
        controller.hide(WindowInsetsCompat.Type.systemBars())
        controller.systemBarsBehavior =
            WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
    }

    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        if (checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        runCatching {
            requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), REQUEST_NOTIFICATIONS)
        }.onFailure { Log.w(TAG, "notification permission request failed", it) }
    }

    private fun showOverlay(title: String, message: String, showRetry: Boolean) {
        overlay.visibility = View.VISIBLE
        overlayVisible.set(true)
        overlayTitle.text = title
        overlayMessage.text = message
        retryButton.visibility = if (showRetry) View.VISIBLE else View.GONE
        runCatching {
            webView.evaluateJavascript("document.body && document.body.classList.add('engine-offline')", null)
        }
    }

    private fun hideOverlay() {
        overlay.visibility = View.GONE
        overlayVisible.set(false)
        runCatching {
            webView.evaluateJavascript("document.body && document.body.classList.remove('engine-offline')", null)
        }
    }

    private companion object {
        const val TAG = "JanusMain"
        const val REQUEST_NOTIFICATIONS = 41
    }
}
