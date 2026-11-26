package com.janus.omega

import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.URI

class RuntimeUrlPolicyTest {
    @Test
    fun remoteRuntimeRequiresHttps() {
        assertNotNull(RuntimeUrlPolicy.parse("https://janus.example:5000", false))
        assertNull(RuntimeUrlPolicy.parse("http://janus.example:5000", false))
    }

    @Test
    fun localHttpAndDebugEmulatorBridgeAreAllowed() {
        assertNotNull(RuntimeUrlPolicy.parse("http://localhost:5000", false))
        assertNotNull(RuntimeUrlPolicy.parse("http://127.0.0.1:5000", false))
        assertNotNull(RuntimeUrlPolicy.parse("http://10.0.2.2:5000", true))
        assertNull(RuntimeUrlPolicy.parse("http://10.0.2.2:5000", false))
    }

    @Test
    fun endpointRejectsCredentialsQueriesAndFragments() {
        assertNull(RuntimeUrlPolicy.parse("https://user:pass@janus.example", false))
        assertNull(RuntimeUrlPolicy.parse("https://janus.example?next=https://other.example", false))
        assertNull(RuntimeUrlPolicy.parse("https://janus.example#session", false))
    }

    @Test
    fun runtimeEndpointRejectsPortZeroAndSubpathDeployment() {
        assertNull(RuntimeUrlPolicy.parse("https://janus.example:0", false))
        assertNull(RuntimeUrlPolicy.parse("https://janus.example/community", false))
        assertNull(RuntimeUrlPolicy.parse("https://janus.example/community/", false))
        assertNotNull(RuntimeUrlPolicy.parse("https://janus.example/", false))
    }

    @Test
    fun originComparisonRejectsHostPrefixSpoofing() {
        val expected = URI("https://janus.example")
        assertFalse(RuntimeUrlPolicy.sameOrigin(expected, "https://janus.example.attacker.invalid/login"))
        assertFalse(RuntimeUrlPolicy.sameOrigin(expected, "https://janus.example:444/login"))
        assertTrue(RuntimeUrlPolicy.sameOrigin(expected, "https://JANUS.example:443/aria"))
    }

    @Test
    fun originComparisonRejectsNonHttpSchemes() {
        assertFalse(RuntimeUrlPolicy.sameOrigin(URI("https://janus.example"), "javascript:alert(1)"))
        assertFalse(RuntimeUrlPolicy.sameOrigin(URI("https://janus.example"), "file:///data/local/tmp/page.html"))
    }
}
