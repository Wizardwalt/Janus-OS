package com.janus.omega.core

import android.content.Context
import android.util.Log
import com.janus.omega.BuildConfig
import java.io.File

/**
 * Unpacks the engine payload that `syncJanusAssets` copied into the APK.
 *
 * The Rust runtime works with real paths (`ServeDir`, `read_dir`, `Connection::open`),
 * so the dashboard, the plugin catalogue and the core modules have to exist on
 * the filesystem before the engine starts. They are unpacked once per install
 * (or app upgrade) into app-private storage and reused afterwards.
 *
 * APK size is the reason this lives on disk: the full catalogue is ~40 MB of Lua
 * and `assets` is not seekable as a directory tree.
 */
object RuntimeAssets {

    private const val TAG = "JanusAssets"
    private const val MARKER = ".payload-stamp"

    /**
     * Directories copied from `assets/` (each one is optional, so a lean build
     * without `modules/` still works).
     */
    private val PAYLOAD_DIRS = listOf("web", "plugins", "core", "modules", "apocalypse_engineering")

    data class Paths(
        val root: File,
        val database: File,
        val web: File,
        val plugins: File,
        val core: File,
    )

    fun prepare(context: Context, onProgress: ((String) -> Unit)? = null): Paths {
        val root = File(context.filesDir, "engine")
        val web = File(root, "web")
        val plugins = File(root, "plugins")
        val core = File(root, "core")

        if (!isCurrent(root, web, plugins, core)) {
            onProgress?.invoke("Unpacking engine payload...")
            root.deleteRecursively()
            if (!root.mkdirs() && !root.isDirectory) {
                throw IllegalStateException("cannot create ${root.absolutePath}")
            }
            PAYLOAD_DIRS.forEach { dir ->
                try {
                    copyAssetTree(context, dir, File(root, dir))
                } catch (error: Throwable) {
                    // An absent directory is normal (lean catalogue builds).
                    Log.i(TAG, "asset '$dir' not unpacked: ${error.message}")
                }
            }
            File(root, MARKER).writeText(stamp())
        }

        return Paths(
            root = root,
            database = File(root, "janus.db"),
            web = web,
            plugins = plugins,
            core = core,
        )
    }

    /** Cheap validity check: right build, and the three mandatory trees exist. */
    private fun isCurrent(root: File, web: File, plugins: File, core: File): Boolean {
        val marker = File(root, MARKER)
        if (!marker.isFile) return false
        val recorded = try {
            marker.readText().trim()
        } catch (error: Throwable) {
            return false
        }
        return recorded == stamp() && web.isDirectory && plugins.isDirectory && core.isDirectory
    }

    /**
     * `VERSION_CODE` changes on every install of a new build and `FULL_CATALOGUE`
     * distinguishes the two payload shapes, so replays only happen when the
     * payload actually changed.
     */
    private fun stamp(): String = "${BuildConfig.VERSION_CODE}-${BuildConfig.FULL_CATALOGUE}"

    private fun copyAssetTree(context: Context, assetPath: String, target: File) {
        val assets = context.assets
        val children = assets.list(assetPath) ?: emptyArray()

        if (children.isEmpty()) {
            // Leaf entry: `list()` returns an empty array for files.
            target.parentFile?.mkdirs()
            assets.open(assetPath).use { input ->
                target.outputStream().use { output -> input.copyTo(output) }
            }
            return
        }

        if (!target.mkdirs() && !target.isDirectory) {
            throw IllegalStateException("cannot create ${target.absolutePath}")
        }
        children.forEach { child ->
            copyAssetTree(context, "$assetPath/$child", File(target, child))
        }
    }
}
