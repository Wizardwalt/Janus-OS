package com.janus.omega

internal data class NormalizedAriaImportMessage(
    val role: String,
    val content: String
)

/**
 * Imported system/developer/tool messages are transcript data, never trusted
 * instructions or tool results in Aria's own conversation context.
 */
internal fun normalizeAriaImportedMessage(sourceRole: String?, content: String): NormalizedAriaImportMessage {
    val role = sourceRole?.trim()?.lowercase().orEmpty()
    return when (role) {
        "assistant", "model" -> NormalizedAriaImportMessage("assistant", content)
        "user", "" -> NormalizedAriaImportMessage("user", content)
        else -> {
            val label = role.filter { it.isLetterOrDigit() || it == '_' || it == '-' }.take(40).ifBlank { "unknown" }
            NormalizedAriaImportMessage("user", "[Imported source role: $label]\n$content")
        }
    }
}
