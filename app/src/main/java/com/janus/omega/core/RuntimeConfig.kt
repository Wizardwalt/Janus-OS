package com.janus.omega.core

import org.json.JSONObject

/**
 * Everything the Rust engine needs to boot, in the shape `janus-android`
 * expects.
 *
 * The JSON keys here are a hard contract with `janus-android/src/lib.rs`
 * (`StartRequest`); renaming one breaks startup at runtime rather than at
 * compile time, so keep them in sync.
 */
data class RuntimeConfig(
    /** Absolute path to the SQLite database; parent directories are created. */
    val dbPath: String,
    /** Directory scanned for Lua plugins. */
    val pluginDir: String,
    /** Directory holding the core module catalogue. */
    val coreDir: String,
    /** Dashboard root served by the engine at `/`. */
    val webDir: String,
    /** Loopback only. The engine must never be exposed on a routable address. */
    val bind: String = "127.0.0.1",
    /** `0` asks the OS for an ephemeral port, which is the kiosk default. */
    val port: Int = 0,
    /** Ed25519 public key (hex/base64) used to verify imported licences. */
    val licensePublicKey: String? = null,
    val auditRetentionDays: Int = 365,
) {
    fun toJson(): String = JSONObject().apply {
        put("db_path", dbPath)
        put("plugin_dir", pluginDir)
        put("core_dir", coreDir)
        put("web_dir", webDir)
        put("bind", bind)
        put("port", port)
        put("audit_retention_days", auditRetentionDays)
        if (!licensePublicKey.isNullOrBlank()) {
            put("license_public_key", licensePublicKey)
        }
    }.toString()
}
