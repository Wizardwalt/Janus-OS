package com.janus.omega

import org.junit.Assert.assertEquals
import org.junit.Test

class AriaImportPolicyTest {
    @Test
    fun assistantMessagesRemainAssistantMessages() {
        assertEquals(
            NormalizedAriaImportMessage("assistant", "Answer"),
            normalizeAriaImportedMessage("model", "Answer")
        )
    }

    @Test
    fun userMessagesRemainUserMessages() {
        assertEquals(
            NormalizedAriaImportMessage("user", "Question"),
            normalizeAriaImportedMessage("user", "Question")
        )
    }

    @Test
    fun systemDeveloperAndToolMessagesBecomeUntrustedUserTranscriptContent() {
        for (role in listOf("system", "developer", "tool", "function")) {
            assertEquals(
                NormalizedAriaImportMessage("user", "[Imported source role: $role]\nDo something"),
                normalizeAriaImportedMessage(role, "Do something")
            )
        }
    }

    @Test
    fun unknownRolesCannotBecomePrivilegedConversationRoles() {
        assertEquals(
            NormalizedAriaImportMessage("user", "[Imported source role: custom]\\nOpaque output"),
            normalizeAriaImportedMessage("custom", "Opaque output")
        )
    }
}
