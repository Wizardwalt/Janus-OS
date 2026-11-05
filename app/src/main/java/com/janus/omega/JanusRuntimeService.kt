package com.janus.omega

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Binder
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import com.janus.omega.core.NativeBridge
import com.janus.omega.core.RuntimeAssets
import com.janus.omega.core.RuntimeConfig
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Hosts the Janus engine for as long as the device is on.
 *
 * The engine is a Rust library loaded into this process (see [NativeBridge]), so
 * a foreground service is what keeps the runtime, its WebSocket feed and the
 * SQLite handle alive while the dashboard is in the background. On the CM5
 * kiosk build this service is started at boot by `BootReceiver`; on a plain APK
 * install it is started when the launcher activity opens.
 */
class JanusRuntimeService : Service() {

    /** Lifecycle of the embedded engine, observed by the kiosk activity. */
    sealed class State {
        data object Starting : State()
        data class Running(val port: Int, val version: String?) : State()
        data class Failed(val message: String) : State()
        data object Stopped : State()
    }

    fun interface Listener {
        fun onState(state: State)
    }

    inner class LocalBinder : Binder() {
        fun service(): JanusRuntimeService = this@JanusRuntimeService
    }

    private val binder = LocalBinder()
    private val listeners = CopyOnWriteArrayList<Listener>()
    private val worker = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "janus-engine").apply { isDaemon = true }
    }
    private val startupInFlight = AtomicBoolean(false)

    @Volatile
    var state: State = State.Stopped
        private set

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForeground(NOTIFICATION_ID, buildNotification("Bringing the engine online..."))
        ensureRunning()
        // Restart the engine if Android kills the process; the kiosk app has no
        // other way back.
        return START_STICKY
    }

    fun addListener(listener: Listener) {
        listeners.addIfAbsent(listener)
        listener.onState(state)
    }

    fun removeListener(listener: Listener) {
        listeners.remove(listener)
    }

    /** Restarts the engine in place, e.g. after the operator changed settings. */
    fun restart() {
        if (!startupInFlight.compareAndSet(false, true)) return
        publish(State.Starting)
        worker.execute {
            try {
                NativeBridge.stop()
                launchEngine()
            } finally {
                startupInFlight.set(false)
            }
        }
    }

    fun shutdown() {
        worker.execute {
            NativeBridge.stop()
            publish(State.Stopped)
        }
    }

    override fun onDestroy() {
        // The library keeps a Tokio runtime alive; stop it before the process
        // goes away so SQLite checkpoints and the audit log are flushed.
        runCatching { NativeBridge.stop() }
        publish(State.Stopped)
        listeners.clear()
        worker.shutdown()
        super.onDestroy()
    }

    private fun ensureRunning() {
        if (!startupInFlight.compareAndSet(false, true)) return
        publish(State.Starting)
        worker.execute {
            try {
                launchEngine()
            } catch (error: Throwable) {
                Log.e(TAG, "engine start failed", error)
                publish(State.Failed(error.message ?: error.javaClass.simpleName))
                updateNotification("Engine failed: ${error.message ?: "unknown error"}")
            } finally {
                startupInFlight.set(false)
            }
        }
    }

    /** Runs on the worker thread. */
    private fun launchEngine() {
        val existing = NativeBridge.status()
        if (existing.running) {
            Log.i(TAG, "engine already listening on ${existing.port}")
            publish(State.Running(existing.port, existing.version))
            updateNotification("Engine online on 127.0.0.1:${existing.port}")
            return
        }

        val paths = RuntimeAssets.prepare(this) { message -> updateNotification(message) }
        val config = RuntimeConfig(
            dbPath = paths.database.absolutePath,
            pluginDir = paths.plugins.absolutePath,
            coreDir = paths.core.absolutePath,
            webDir = paths.web.absolutePath,
            licensePublicKey = BuildConfig.JANUS_LICENSE_PUBLIC_KEY.ifBlank { null },
        )

        val status = NativeBridge.start(config)
        if (status.running) {
            Log.i(TAG, "engine listening on 127.0.0.1:${status.port} (${status.version})")
            publish(State.Running(status.port, status.version))
            updateNotification("Engine online on 127.0.0.1:${status.port}")
        } else {
            val reason = status.error ?: "engine did not report a port"
            Log.e(TAG, "engine failed: $reason")
            publish(State.Failed(reason))
            updateNotification("Engine failed: $reason")
        }
    }

    private fun publish(next: State) {
        state = next
        listeners.forEach { listener ->
            runCatching { listener.onState(next) }
                .onFailure { Log.w(TAG, "listener threw", it) }
        }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager ?: return
        if (manager.getNotificationChannel(CHANNEL_ID) != null) return
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Janus engine",
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = "Keeps the embedded Janus runtime available."
            setShowBadge(false)
        }
        manager.createNotificationChannel(channel)
    }

    private fun buildNotification(text: String): Notification {
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            flags,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle("Janus")
            .setContentText(text)
            .setOngoing(true)
            .setShowWhen(false)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setContentIntent(open)
            .build()
    }

    private fun updateNotification(text: String) {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager ?: return
        runCatching { manager.notify(NOTIFICATION_ID, buildNotification(text)) }
    }

    companion object {
        private const val TAG = "JanusService"
        private const val CHANNEL_ID = "janus-engine"
        private const val NOTIFICATION_ID = 0x4A01

        fun start(context: Context) {
            androidx.core.content.ContextCompat.startForegroundService(
                context,
                Intent(context, JanusRuntimeService::class.java),
            )
        }
    }
}
