package com.janus.omega

import android.annotation.SuppressLint
import android.app.AlertDialog
import android.os.Bundle
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.EditText
import androidx.appcompat.app.AppCompatActivity
import org.json.JSONObject
import java.net.URI

/**
 * Imports one user-approved OmniVault snapshot into Aria's encrypted
 * conversation history. The Janus bearer token remains inside WebView
 * sessionStorage and is never copied into native storage or OmniVault.
 */
class AriaImportActivity : AppCompatActivity() {
    private var webView: WebView? = null
    private var injected = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val title = intent.getStringExtra(EXTRA_TITLE)
        val snapshotJson = intent.getStringExtra(EXTRA_SNAPSHOT)
        val conversationId = intent.getStringExtra(EXTRA_CONVERSATION_ID)
        if (title.isNullOrBlank() || snapshotJson.isNullOrBlank() || conversationId.isNullOrBlank()) {
            finishWithMessage("The OmniVault transfer is incomplete.")
            return
        }
        if (snapshotJson.toByteArray(Charsets.UTF_8).size > MAX_TRANSFER_BYTES) {
            finishWithMessage("The OmniVault transfer exceeds Janus's 512 KiB limit.")
            return
        }
        val snapshot = runCatching { JSONObject(snapshotJson) }.getOrNull()
        if (snapshot?.optString("schema") != "omnivault.aria-transfer.v1" ||
            snapshot.optJSONArray("messages") == null
        ) {
            finishWithMessage("The OmniVault transfer format is not supported.")
            return
        }

        // Imported provider prompts/tool output are untrusted transcript content.
        // Never promote them into Aria's system/developer/tool message roles.
        val messages = snapshot.optJSONArray("messages")!!
        for (index in 0 until messages.length()) {
            val message = messages.optJSONObject(index)
                ?: run {
                    finishWithMessage("The OmniVault transfer contains an invalid message.")
                    return
                }
            val normalized = normalizeAriaImportedMessage(
                message.optString("role"),
                message.optString("content")
            )
            message.put("role", normalized.role)
            message.put("content", normalized.content)
        }
        val safeSnapshotJson = snapshot.toString()
        if (safeSnapshotJson.toByteArray(Charsets.UTF_8).size > MAX_TRANSFER_BYTES) {
            finishWithMessage("The normalized transfer exceeds Janus's 512 KiB limit.")
            return
        }

        val prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
        val savedUrl = prefs.getString(KEY_SERVER_URL, DEFAULT_SERVER_URL).orEmpty()
        val input = EditText(this).apply {
            setSingleLine(true)
            setText(savedUrl)
            hint = "https://your-janus-runtime:5000"
        }
        AlertDialog.Builder(this)
            .setTitle("Connect to Janus runtime")
            .setMessage(
                "Choose the Janus runtime address. The selected conversation will be imported into " +
                    "the signed-in account's encrypted Aria conversation history. Only this snapshot is sent. " +
                    "No Janus password or bearer token is shared with OmniVault."
            )
            .setView(input)
            .setPositiveButton("Connect and import") { _, _ ->
                val serverUrl = input.text.toString().trim().trimEnd('/')
                val allowEmulatorHost = (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0
                val parsed = RuntimeUrlPolicy.parse(serverUrl, allowEmulatorHost)
                if (parsed == null) {
                    finishWithMessage(
                        "Remote Janus runtimes must use HTTPS. HTTP is allowed only for localhost " +
                            "(and 10.0.2.2 in debug emulator builds)."
                    )
                } else {
                    val normalizedServerUrl = parsed.toString().trimEnd('/')
                    prefs.edit().putString(KEY_SERVER_URL, normalizedServerUrl).apply()
                    openRuntime(normalizedServerUrl, parsed, title, safeSnapshotJson, conversationId)
                }
            }
            .setNegativeButton("Cancel") { _, _ -> finish() }
            .setOnCancelListener { finish() }
            .show()
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun openRuntime(serverUrl: String, runtimeUri: URI, title: String, snapshotJson: String, conversationId: String) {
        val view = WebView(this)
        webView = view
        view.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            allowFileAccess = false
            allowContentAccess = false
            mixedContentMode = android.webkit.WebSettings.MIXED_CONTENT_NEVER_ALLOW
        }
        view.webChromeClient = WebChromeClient()
        view.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                return !RuntimeUrlPolicy.sameOrigin(runtimeUri, request.url.toString())
            }

            override fun onPageStarted(view: WebView, url: String, favicon: android.graphics.Bitmap?) {
                if (RuntimeUrlPolicy.sameOrigin(runtimeUri, url)) injected = false
            }

            override fun onPageFinished(view: WebView, url: String) {
                if (!injected && RuntimeUrlPolicy.sameOrigin(runtimeUri, url)) {
                    injected = true
                    submitAfterLogin(view, title, snapshotJson, conversationId)
                }
            }
        }
        setContentView(view)
        view.loadUrl(serverUrl)
    }

    /**
     * Reconstruct the selected message sequence and append it idempotently to
     * the owner's encrypted ConversationStore. Existing event IDs are skipped,
     * so retrying a partial import does not duplicate already stored messages.
     */
    private fun submitAfterLogin(
        view: WebView,
        title: String,
        snapshotJson: String,
        conversationId: String
    ) {
        val safeTitle = JSONObject.quote(title)
        val safeSnapshot = JSONObject.quote(snapshotJson)
        val safeConversationId = JSONObject.quote(conversationId)
        val script = """
            (function () {
              const title = $safeTitle;
              const snapshot = JSON.parse($safeSnapshot);
              const conversationId = $safeConversationId;
              const sourceProvider = String(snapshot.provider || 'Unknown').slice(0, 80);
              const provider = ('OmniVault / ' + sourceProvider).slice(0, 128);
              const digest = conversationId.replace(/^omnivault_/, '');
              const messages = Array.isArray(snapshot.messages) ? snapshot.messages : [];
              const maxChunkBytes = 60000;

              function splitUtf8(text) {
                const encoder = new TextEncoder();
                const chunks = [];
                let current = '';
                let currentBytes = 0;
                for (const character of text) {
                  const size = encoder.encode(character).length;
                  if (currentBytes + size > maxChunkBytes && current.length > 0) {
                    chunks.push(current);
                    current = '';
                    currentBytes = 0;
                  }
                  current += character;
                  currentBytes += size;
                }
                if (current.length > 0) chunks.push(current);
                return chunks;
              }

              function normalizedRole(value) {
                const role = String(value || '').toLowerCase();
                if (role === 'assistant') return 'assistant';
                return 'user';
              }

              function eventTime(value) {
                if (typeof value !== 'string') return null;
                const parsed = Date.parse(value);
                return Number.isFinite(parsed) ? new Date(parsed).toISOString() : null;
              }

              async function requestJson(url, options) {
                const response = await fetch(url, options);
                const payload = await response.json().catch(() => ({}));
                if (!response.ok) {
                  throw new Error(payload.error || ('Janus runtime returned HTTP ' + response.status));
                }
                return payload;
              }

              async function submitWhenSignedIn() {
                let attempts = 0;
                while (!sessionStorage.getItem('janus.sessionToken') && attempts < 180) {
                  attempts += 1;
                  await new Promise(resolve => setTimeout(resolve, 1000));
                }
                const token = sessionStorage.getItem('janus.sessionToken');
                if (!token) {
                  alert('Janus sign-in was not detected. No conversation was imported. Sign in and share again.');
                  return;
                }

                const base = location.origin;
                const headers = {
                  'Content-Type': 'application/json',
                  'Authorization': 'Bearer ' + token
                };
                const endpoint = base + '/aria/conversations/' + encodeURIComponent(conversationId) + '/events';
                const existingIds = new Set();
                let cursor = null;
                while (true) {
                  const query = '?limit=1000' + (cursor ? '&after_event_id=' + encodeURIComponent(cursor) : '');
                  const existingPayload = await requestJson(endpoint + query, { method: 'GET', headers: headers });
                  const page = Array.isArray(existingPayload.data) ? existingPayload.data : [];
                  page.forEach(event => existingIds.add(event.event_id));
                  if (page.length < 1000) break;
                  cursor = page[page.length - 1].event_id;
                }

                let imported = 0;
                let skipped = 0;
                const metadataId = 'ov_' + digest + '_0000_0000';
                if (!existingIds.has(metadataId)) {
                  const metadata = {
                    event_id: metadataId,
                    role: 'system',
                    content: 'Imported from OmniVault. Conversation title: ' + title +
                      '. Source provider: ' + sourceProvider +
                      '. Original source conversation reference: ' + String(snapshot.id || '').slice(0, 160) + '.',
                    provider: provider,
                    model: null,
                    created_at: eventTime(snapshot.created_at)
                  };
                  await requestJson(endpoint, {
                    method: 'POST',
                    headers: headers,
                    body: JSON.stringify(metadata)
                  });
                  existingIds.add(metadataId);
                  imported += 1;
                } else {
                  skipped += 1;
                }

                for (let index = 0; index < messages.length; index += 1) {
                  const message = messages[index] || {};
                  const content = String(message.content || '');
                  if (!content.trim()) continue;
                  const pieces = splitUtf8(content);
                  for (let part = 0; part < pieces.length; part += 1) {
                    const eventId = 'ov_' + digest + '_' +
                      String(index + 1).padStart(4, '0') + '_' +
                      String(part).padStart(4, '0');
                    if (existingIds.has(eventId)) {
                      skipped += 1;
                      continue;
                    }
                    const event = {
                      event_id: eventId,
                      role: normalizedRole(message.role),
                      content: pieces[part],
                      provider: provider,
                      model: null,
                      created_at: eventTime(message.timestamp)
                    };
                    await requestJson(endpoint, {
                      method: 'POST',
                      headers: headers,
                      body: JSON.stringify(event)
                    });
                    existingIds.add(eventId);
                    imported += 1;
                  }
                }
                alert('Aria conversation import complete. Added ' + imported +
                  ' event(s); skipped ' + skipped + ' previously imported event(s).');
              }

              submitWhenSignedIn().catch(error => {
                alert('Aria conversation import failed: ' +
                  (error && error.message ? error.message : 'Janus runtime unavailable.'));
              });
            })();
        """.trimIndent()
        view.evaluateJavascript(script, null)
    }

    private fun finishWithMessage(message: String) {
        AlertDialog.Builder(this)
            .setTitle("Aria import unavailable")
            .setMessage(message)
            .setPositiveButton("OK") { _, _ -> finish() }
            .setOnCancelListener { finish() }
            .show()
    }

    override fun onDestroy() {
        webView?.stopLoading()
        webView?.destroy()
        webView = null
        super.onDestroy()
    }

    companion object {
        const val EXTRA_TITLE = "com.janus.omega.extra.OMNIVAULT_TITLE"
        const val EXTRA_SNAPSHOT = "com.janus.omega.extra.OMNIVAULT_SNAPSHOT"
        const val EXTRA_CONVERSATION_ID = "com.janus.omega.extra.OMNIVAULT_CONVERSATION_ID"
        private const val PREFS = "janus_prefs"
        private const val KEY_SERVER_URL = "server_url"
        private const val DEFAULT_SERVER_URL = ""
        private const val MAX_TRANSFER_BYTES = 512 * 1024
    }
}
