package com.janus.omega

import java.net.URI

/**
 * Runtime endpoints may carry a user's Janus sign-in session. Require TLS for
 * remote hosts and compare parsed origins rather than URL string prefixes.
 */
internal object RuntimeUrlPolicy {
    fun parse(raw: String, allowEmulatorHost: Boolean): URI? {
        val candidate = raw.trim().trimEnd('/')
        val uri = runCatching { URI(candidate) }.getOrNull() ?: return null
        if (!uri.isAbsolute || uri.isOpaque) return null

        val scheme = normalizeScheme(uri.scheme)
        if (scheme != "https" && scheme != "http") return null
        val host = normalizeHost(uri.host ?: return null)
        if (host.isBlank() || uri.rawUserInfo != null || uri.rawQuery != null || uri.rawFragment != null) {
            return null
        }
        if (uri.port !in -1..65535 || uri.port == 0) return null
        // Runtime API calls are rooted at location.origin. Reject sub-path
        // deployments rather than silently sending authenticated requests to
        // the wrong application root.
        if (!uri.rawPath.isNullOrEmpty() && uri.rawPath != "/") return null

        if (scheme == "http") {
            val loopback = host in setOf("localhost", "127.0.0.1", "::1")
            val emulatorBridge = allowEmulatorHost && host == "10.0.2.2"
            if (!loopback && !emulatorBridge) return null
        }
        return uri
    }

    fun sameOrigin(expected: URI, candidateUrl: String): Boolean {
        val candidate = runCatching { URI(candidateUrl) }.getOrNull() ?: return false
        if (!candidate.isAbsolute || candidate.isOpaque || candidate.host.isNullOrBlank()) return false
        if (candidate.rawUserInfo != null) return false

        return normalizeScheme(expected.scheme) == normalizeScheme(candidate.scheme) &&
            normalizeHost(expected.host ?: return false) == normalizeHost(candidate.host) &&
            effectivePort(expected) == effectivePort(candidate)
    }

    private fun effectivePort(uri: URI): Int =
        uri.port.takeIf { it != -1 } ?: when (normalizeScheme(uri.scheme)) {
            "https" -> 443
            "http" -> 80
            else -> -1
        }

    private fun normalizeHost(host: String): String =
        host.lowercase().removePrefix("[").removeSuffix("]")

    private fun normalizeScheme(scheme: String?): String = scheme?.lowercase().orEmpty()
}
