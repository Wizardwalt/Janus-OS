package com.janus.omega.core

import android.util.Log
import org.json.JSONObject

/**
 * Result of a call into the native engine.
 *
 * The JNI layer answers every call with a JSON object so a failure inside Rust
 * (including a panic caught by `catch_unwind`) still produces a structured
 * answer instead of a process abort:
 *
 *   `{"ok":true,"port":41234,"version":"0.1.0"}`
 *   `{"ok":false,"error":"engine already running"}`
 */
data class EngineStatus(
    val running: Boolean,
    val port: Int,
    val version: String?,
    val error: String?,
) {
    companion object {
        fun failure(message: String) = EngineStatus(false, 0, null, message)
    }
}

/**
 * The only entry point into `libjanus_android.so`.
 *
 * Nothing in the app talks to the engine except through this class, which keeps
 * the JNI symbol names in one place: the native methods are instance methods of
 * this class, so the Rust exports are
 * `Java_com_janus_omega_core_NativeBridge_nativeStart` (and `…Stop`, `…Status`)
 * with a `jobject` receiver. Do not move them into the companion object or a
 * file-level declaration — that changes the exported symbol name to
 * `…_00024Companion_…` and the library will fail to resolve them at runtime.
 */
class NativeBridge private constructor() {

    private external fun nativeStart(configJson: String): String

    private external fun nativeStop(): String

    private external fun nativeStatus(): String

    companion object {
        private const val TAG = "JanusNative"
        private const val LIBRARY = "janus_android"

        private val instance = NativeBridge()

        @Volatile
        private var loaded = false

        @Volatile
        private var loadError: String? = null

        /**
         * Loads the shared object once. A missing ABI build (for example an
         * APK installed on a device whose ABI was not packaged) surfaces here as
         * a plain `false` so the UI can explain it instead of crashing.
         */
        @Synchronized
        fun load(): Boolean {
            if (loaded) return true
            return try {
                System.loadLibrary(LIBRARY)
                loaded = true
                true
            } catch (error: Throwable) {
                loadError = error.message ?: error.javaClass.simpleName
                Log.e(TAG, "Cannot load lib$LIBRARY.so", error)
                false
            }
        }

        fun unavailableReason(): String =
            loadError ?: "lib$LIBRARY.so is not packaged for this ABI"

        fun start(config: RuntimeConfig): EngineStatus {
            if (!load()) return EngineStatus.failure(unavailableReason())
            return parse { instance.nativeStart(config.toJson()) }
        }

        fun stop(): EngineStatus {
            if (!load()) return EngineStatus.failure(unavailableReason())
            return parse { instance.nativeStop() }
        }

        fun status(): EngineStatus {
            if (!load()) return EngineStatus.failure(unavailableReason())
            return parse { instance.nativeStatus() }
        }

        private fun parse(call: () -> String): EngineStatus {
            val payload = try {
                call()
            } catch (error: Throwable) {
                Log.e(TAG, "native call failed", error)
                return EngineStatus.failure(error.message ?: error.javaClass.simpleName)
            }

            val json = try {
                JSONObject(payload)
            } catch (error: Throwable) {
                return EngineStatus.failure("malformed engine response: $payload")
            }

            val ok = json.optBoolean("ok", false)
            val port = if (json.has("port") && !json.isNull("port")) json.optInt("port", 0) else 0
            val version = if (json.has("version") && !json.isNull("version")) json.optString("version") else null
            val error = if (json.has("error") && !json.isNull("error")) json.optString("error") else null

            return EngineStatus(
                running = ok && port > 0,
                port = port,
                version = version,
                error = error ?: if (ok) null else "engine reported failure",
            )
        }
    }
}
