package com.janus.omega

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ModuleDataTest {
    @Test
    fun catalogCountMatchesTheActualUniqueEntries() {
        val entries = ModuleData.categories.flatMap { it.second }

        assertEquals(8, ModuleData.categories.size)
        assertEquals(257, entries.size)
        assertEquals(entries.size, entries.toSet().size)
        assertEquals(entries.size, ModuleData.totalEntries)
        assertTrue(ModuleData.categories.all { (_, modules) -> modules.isNotEmpty() })
    }
}
