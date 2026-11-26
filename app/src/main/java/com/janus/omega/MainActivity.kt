package com.janus.omega

import android.app.AlertDialog
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.view.WindowManager
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.viewpager2.widget.ViewPager2
import com.google.android.material.tabs.TabLayout
import com.google.android.material.tabs.TabLayoutMediator
import org.json.JSONObject
import java.security.MessageDigest

class MainActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        setContentView(R.layout.activity_main)

        val viewPager = findViewById<ViewPager2>(R.id.view_pager)
        val tabLayout = findViewById<TabLayout>(R.id.tab_layout)

        viewPager.adapter = TacticalPagerAdapter(this)

        TabLayoutMediator(tabLayout, viewPager) { tab, position ->
            tab.text = when (position) {
                0 -> "DASHBOARD"
                1 -> "OPS"
                2 -> "HARDWARE"
                else -> ""
            }
        }.attach()

        handleOmniVaultShare(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleOmniVaultShare(intent)
    }

    private fun handleOmniVaultShare(incoming: Intent?) {
        if (incoming?.action != Intent.ACTION_SEND || incoming.type != "application/json") return
        val uri = incoming.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)
            ?: incoming.clipData?.getItemAt(0)?.uri
        if (uri == null) {
            Toast.makeText(this, "OmniVault did not provide a readable snapshot.", Toast.LENGTH_LONG).show()
            return
        }

        AlertDialog.Builder(this)
            .setTitle("Import one conversation into Aria?")
            .setMessage(
                "OmniVault is offering one read-only conversation snapshot. " +
                    "If you continue, Janus will read only this snapshot, create a transcript, " +
                    "and send it to the configured Janus runtime as a private artifact for the " +
                    "currently signed-in account. The original OmniVault archive and raw provider " +
                    "source record are not transferred. Cancel to import nothing."
            )
            .setPositiveButton("Review and import") { _, _ -> importOmniVaultSnapshot(uri) }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun importOmniVaultSnapshot(uri: Uri) {
        try {
            val bytes = contentResolver.openInputStream(uri)?.use { input ->
                val output = java.io.ByteArrayOutputStream()
                val buffer = ByteArray(8192)
                var total = 0
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    total += count
                    require(total <= MAX_TRANSFER_BYTES) {
                        "The selected conversation exceeds Janus's 512 KiB import limit."
                    }
                    output.write(buffer, 0, count)
                }
                output.toByteArray()
            } ?: error("Janus could not open the OmniVault snapshot.")

            val snapshot = JSONObject(bytes.toString(Charsets.UTF_8))
            require(snapshot.optString("schema") == "omnivault.aria-transfer.v1") {
                "Unsupported OmniVault transfer format."
            }
            val conversationId = snapshot.optString("id")
            val title = snapshot.optString("title").take(100)
            val provider = snapshot.optString("provider").take(80)
            val messages = snapshot.optJSONArray("messages")
                ?: error("The selected snapshot has no messages.")
            require(conversationId.isNotBlank() && title.isNotBlank() && messages.length() > 0) {
                "The selected snapshot is incomplete."
            }

            require(messages.length() in 1..MAX_IMPORTED_MESSAGES) {
                "The transfer must contain between 1 and $MAX_IMPORTED_MESSAGES messages."
            }
            val transferJson = snapshot.toString()
            require(transferJson.toByteArray(Charsets.UTF_8).size <= MAX_TRANSFER_BYTES) {
                "The selected conversation exceeds the 512 KiB runtime import limit."
            }
            // Hash the entire normalized snapshot, not just provider/id: fallback
            // IDs such as "import-1" can legitimately repeat across source archives.
            val digest = MessageDigest.getInstance("SHA-256")
                .digest(transferJson.toByteArray(Charsets.UTF_8))
                .joinToString("") { "%02x".format(it) }
                .take(32)
            val janusConversationId = "omnivault_$digest"

            startActivity(Intent(this, AriaImportActivity::class.java).apply {
                putExtra(AriaImportActivity.EXTRA_TITLE, "OmniVault — $title")
                putExtra(AriaImportActivity.EXTRA_SNAPSHOT, transferJson)
                putExtra(AriaImportActivity.EXTRA_CONVERSATION_ID, janusConversationId)
            })
        } catch (error: Exception) {
            AlertDialog.Builder(this)
                .setTitle("Aria import could not start")
                .setMessage(error.message ?: "The selected snapshot could not be read.")
                .setPositiveButton("OK", null)
                .show()
        }
    }

    companion object {
        private const val MAX_TRANSFER_BYTES = 512 * 1024
        private const val MAX_IMPORTED_MESSAGES = 1000
    }
}
