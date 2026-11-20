package com.janus.omega

import android.app.AlertDialog
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.widget.Button
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import org.json.JSONObject
import kotlin.concurrent.thread

class HomeActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_home)

        val statusText = findViewById<TextView>(R.id.statusText)
        val operatorButton = findViewById<Button>(R.id.operatorButton)
        val modulesButton = findViewById<Button>(R.id.modulesButton)
        val assistantButton = findViewById<Button>(R.id.assistantButton)
        val diagnosticsButton = findViewById<Button>(R.id.diagnosticsButton)
        val notesButton = findViewById<Button>(R.id.notesButton)
        val evidenceButton = findViewById<Button>(R.id.evidenceButton)
        val settingsButton = findViewById<Button>(R.id.settingsButton)

        startService(Intent(this, JanusForegroundService::class.java))

        val fullInterfaceButton = findViewById<Button>(R.id.fullInterfaceButton)
        fullInterfaceButton.setOnClickListener {
            startActivity(Intent(this, JanusWebActivity::class.java))
        }

        operatorButton.setOnClickListener {
            startActivity(Intent(this, OperatorActivity::class.java))
        }

        modulesButton.setOnClickListener {
            startActivity(Intent(this, ModulesActivity::class.java))
        }

        assistantButton.setOnClickListener {
            startActivity(Intent(this, ChatActivity::class.java))
        }

        diagnosticsButton.setOnClickListener {
            startActivity(Intent(this, DiagnosticsActivity::class.java))
        }

        notesButton.setOnClickListener {
            startActivity(Intent(this, NotesActivity::class.java))
        }

        evidenceButton.setOnClickListener {
            startActivity(Intent(this, EvidenceActivity::class.java))
        }

        settingsButton.setOnClickListener {
            startActivity(Intent(this, SettingsActivity::class.java))
        }

        handleAriaShareIntent(intent)

        thread {
            val result = RuntimeClient.get("/api/status")
            runOnUiThread {
                statusText.text = "Runtime status:\n$result"
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleAriaShareIntent(intent)
    }

    /**
     * OmniVault grants this app temporary read access to exactly one selected
     * snapshot. Janus asks for a second, independent confirmation before
     * reading it or sending its transcript to the configured runtime.
     */
    private fun handleAriaShareIntent(incoming: Intent?) {
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
                    "If you continue, Janus will read only this snapshot, create a text transcript, " +
                    "and send it to the configured Janus runtime as a private artifact for the " +
                    "currently signed-in account. The original OmniVault archive is not transferred. " +
                    "You can cancel without importing anything."
            )
            .setPositiveButton("Review and import") { _, _ -> importAriaSnapshot(uri) }
            .setNegativeButton("Cancel") { _, _ ->
                Toast.makeText(this, "Aria import cancelled.", Toast.LENGTH_SHORT).show()
            }
            .show()
    }

    private fun importAriaSnapshot(uri: Uri) {
        try {
            val bytes = contentResolver.openInputStream(uri)?.use { input ->
                val output = java.io.ByteArrayOutputStream()
                val buffer = ByteArray(8192)
                var total = 0
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    total += count
                    require(total <= MAX_ARIA_TRANSFER_BYTES) {
                        "The selected conversation exceeds Janus's 1 MiB import limit."
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

            val transcript = buildString {
                appendLine("Source: OmniVault user-approved conversation transfer")
                appendLine("Provider: $provider")
                appendLine("Conversation reference: $conversationId")
                appendLine()
                for (index in 0 until messages.length()) {
                    val message = messages.optJSONObject(index) ?: continue
                    val role = message.optString("role", "unknown").take(40)
                    val text = message.optString("content")
                    if (text.isNotBlank()) {
                        append(role.uppercase())
                        append(": ")
                        appendLine(text)
                        appendLine()
                    }
                }
            }
            require(transcript.isNotBlank() && transcript.toByteArray(Charsets.UTF_8).size <= MAX_ARIA_TRANSFER_BYTES) {
                "The conversation transcript is empty or exceeds the 1 MiB runtime limit."
            }

            val openAria = Intent(this, JanusWebActivity::class.java).apply {
                putExtra(EXTRA_ARIA_IMPORT_TITLE, "OmniVault — $title")
                putExtra(EXTRA_ARIA_IMPORT_TEXT, transcript)
            }
            startActivity(openAria)
        } catch (error: Exception) {
            AlertDialog.Builder(this)
                .setTitle("Aria import could not start")
                .setMessage(error.message ?: "The selected snapshot could not be read.")
                .setPositiveButton("OK", null)
                .show()
        }
    }

    companion object {
        const val EXTRA_ARIA_IMPORT_TITLE = "com.janus.omega.extra.ARIA_IMPORT_TITLE"
        const val EXTRA_ARIA_IMPORT_TEXT = "com.janus.omega.extra.ARIA_IMPORT_TEXT"
        private const val MAX_ARIA_TRANSFER_BYTES = 1024 * 1024
    }
}
