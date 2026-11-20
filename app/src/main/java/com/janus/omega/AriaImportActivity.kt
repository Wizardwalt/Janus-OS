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
 * Imports one user-approved OmniVault transcript through the existing
 * authenticated Janus runtime API. The bearer token stays inside WebView
 * sessionStorage and is never copied into native app storage or OmniVault.
 */
class AriaImportActivity : AppCompatActivity() {
    private var webView: WebView? = null
    private var injected = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val title = intent.getStringExtra(EXTRA_TITLE)
        val text = intent.getStringExtra(EXTRA_TEXT)
        if (title.isNullOrBlank() || text.isNullOrBlank()) {
            finishWithMessage("The OmniVault transfer is incomplete.")
            return
        }
        if (text.toByteArray(Charsets.UTF_8).size > MAX_TEXT_BYTES || title.toByteArray(Charsets.UTF_8).size > 500) {
            finishWithMessage("The OmniVault transfer exceeds Janus's import limit.")
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
                "Choose the Janus runtime address. The selected transcript will be sent only to this address, " +
                    "using the currently signed-in Janus session. No Janus password or bearer token is shared with OmniVault."
            )
            .setView(input)
            .setPositiveButton("Connect and import") { _, _ ->
                val serverUrl = input.text.toString().trim().trimEnd('/')
                val parsed = runCatching { URI(serverUrl) }.getOrNull()
                if (parsed == null || parsed.scheme !in setOf("http", "https") || parsed.host.isNullOrBlank()) {
                    finishWithMessage("Enter a valid http:// or https:// Janus runtime URL.")
                } else {
                    prefs.edit().putString(KEY_SERVER_URL, serverUrl).apply()
                    openRuntime(serverUrl, title, text)
                }
            }
            .setNegativeButton("Cancel") { _, _ -> finish() }
            .setOnCancelListener { finish() }
            .show()
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun openRuntime(serverUrl: String, title: String, text: String) {
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
                val destination = request.url.toString()
                return !destination.startsWith(serverUrl)
            }

            override fun onPageFinished(view: WebView, url: String) {
                if (!injected && url.startsWith(serverUrl)) {
                    injected = true
                    submitAfterLogin(view, title, text)
                }
            }
        }
        setContentView(view)
        view.loadUrl(serverUrl)
    }

    private fun submitAfterLogin(view: WebView, title: String, text: String) {
        val safeTitle = JSONObject.quote(title)
        val safeText = JSONObject.quote(text)
        val script = """
            (function () {
              const title = $safeTitle;
              const text = $safeText;
              let attempts = 0;
              async function submitWhenSignedIn() {
                const token = sessionStorage.getItem('janus.sessionToken');
                if (!token) {
                  attempts += 1;
                  if (attempts < 180) {
                    setTimeout(submitWhenSignedIn, 1000);
                  } else {
                    alert('Janus sign-in was not detected. No transcript was imported. Sign in and share the conversation again.');
                  }
                  return;
                }
                const base = (localStorage.getItem('janus.runtimeUrl') || location.origin).replace(/\/+$/, '');
                try {
                  const response = await fetch(base + '/aria/artifacts/text', {
                    method: 'POST',
                    headers: {
                      'Content-Type': 'application/json',
                      'Authorization': 'Bearer ' + token
                    },
                    body: JSON.stringify({ title: title, text: text })
                  });
                  const payload = await response.json().catch(() => ({}));
                  if (!response.ok) {
                    throw new Error(payload.error || ('Runtime returned HTTP ' + response.status));
                  }
                  alert('Imported into Aria as a private knowledge artifact.');
                } catch (error) {
                  alert('Aria import failed: ' + (error && error.message ? error.message : 'Runtime unavailable.'));
                }
              }
              submitWhenSignedIn();
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
        const val EXTRA_TEXT = "com.janus.omega.extra.OMNIVAULT_TEXT"
        private const val PREFS = "janus_prefs"
        private const val KEY_SERVER_URL = "server_url"
        private const val DEFAULT_SERVER_URL = "http://10.0.2.2:5000"
        private const val MAX_TEXT_BYTES = 1024 * 1024
    }
}
