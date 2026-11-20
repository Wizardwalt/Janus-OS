package com.janus.omega

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.os.Bundle
import android.view.KeyEvent
import android.webkit.*
import android.widget.FrameLayout
import androidx.appcompat.app.AppCompatActivity
import org.json.JSONObject

class JanusWebActivity : AppCompatActivity() {

    private lateinit var webView: WebView
    private lateinit var prefs: android.content.SharedPreferences
    private var pendingAriaImport: Pair<String, String>? = null
    private var ariaImportInjectionStarted = false

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        pendingAriaImport = intent.getStringExtra(HomeActivity.EXTRA_ARIA_IMPORT_TITLE)?.let { title ->
            intent.getStringExtra(HomeActivity.EXTRA_ARIA_IMPORT_TEXT)?.let { text -> title to text }
        }
        prefs = getSharedPreferences("janus_prefs", Context.MODE_PRIVATE)

        val frame = FrameLayout(this)
        frame.setBackgroundColor(Color.parseColor("#08110B"))
        setContentView(frame)

        webView = WebView(this)
        webView.layoutParams = FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.MATCH_PARENT,
            FrameLayout.LayoutParams.MATCH_PARENT
        )
        frame.addView(webView)

        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            allowFileAccess = true
            mediaPlaybackRequiresUserGesture = false
            @Suppress("DEPRECATION")
            mixedContentMode = WebSettings.MIXED_CONTENT_ALWAYS_ALLOW
            useWideViewPort = true
            loadWithOverviewMode = true
            setSupportZoom(false)
            builtInZoomControls = false
            displayZoomControls = false
            cacheMode = WebSettings.LOAD_DEFAULT
        }

        webView.setBackgroundColor(Color.parseColor("#08110B"))

        webView.webViewClient = object : WebViewClient() {
            override fun onPageFinished(view: WebView, url: String) {
                view.evaluateJavascript(
                    "if (!document.querySelector('meta[name=viewport]')) {" +
                    "var m=document.createElement('meta');m.name='viewport';" +
                    "m.content='width=device-width,initial-scale=1,maximum-scale=1';" +
                    "document.head.appendChild(m);}", null
                )
                if (url.startsWith(getServerUrl().trimEnd('/'))) {
                    startPendingAriaImport(view)
                }
            }

            override fun onReceivedError(
                view: WebView, request: WebResourceRequest, error: WebResourceError
            ) {
                if (request.isForMainFrame) {
                    val url = getServerUrl()
                    view.loadData(errorPage(url), "text/html", "utf-8")
                }
            }
        }

        webView.webChromeClient = WebChromeClient()

        webView.loadUrl(getServerUrl())
    }

    /**
     * Keep the Janus bearer token inside the authenticated WebView session.
     * OmniVault never receives or handles Janus credentials. The import is
     * attempted only after the user has an active Janus session.
     */
    private fun startPendingAriaImport(view: WebView) {
        val pending = pendingAriaImport ?: return
        if (ariaImportInjectionStarted) return
        ariaImportInjectionStarted = true
        pendingAriaImport = null
        intent.removeExtra(HomeActivity.EXTRA_ARIA_IMPORT_TITLE)
        intent.removeExtra(HomeActivity.EXTRA_ARIA_IMPORT_TEXT)

        val title = JSONObject.quote(pending.first)
        val text = JSONObject.quote(pending.second)
        val script = """
            (function () {
              const title = $title;
              const text = $text;
              let attempts = 0;
              async function submitWhenSignedIn() {
                const token = sessionStorage.getItem('janus.sessionToken');
                if (!token) {
                  attempts += 1;
                  if (attempts < 120) {
                    setTimeout(submitWhenSignedIn, 1000);
                  } else {
                    alert('OmniVault transfer is ready, but Janus sign-in was not detected. Sign in and share the conversation again.');
                  }
                  return;
                }
                const base = (localStorage.getItem('janus.runtimeUrl') || location.origin).replace(//+$/, '');
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

    private fun getServerUrl(): String {
        return prefs.getString("server_url", "http://10.0.2.2:5000") ?: "http://10.0.2.2:5000"
    }

    private fun errorPage(url: String) = """
        <!DOCTYPE html>
        <html>
        <head>
          <meta name="viewport" content="width=device-width,initial-scale=1">
          <style>
            body { background:#08110B; color:#62FF8F; font-family:monospace; padding:24px; margin:0; }
            h2   { color:#9D00FF; margin-bottom:16px; }
            code { background:#102018; padding:4px 8px; border-radius:4px; }
            .btn { display:inline-block; margin-top:20px; background:#62FF8F; color:#000;
                   padding:12px 24px; border:none; cursor:pointer; font-size:16px;
                   font-family:monospace; font-weight:bold; text-decoration:none; }
          </style>
        </head>
        <body>
          <h2>⚡ JANUS-OS — NO SIGNAL</h2>
          <p>Cannot reach server at:<br><code>$url</code></p>
          <p>Make sure <code>janus-web</code> is running on your Pandora unit and both devices share the same network.</p>
          <p>Start the server: <code>cargo run --bin janus-web</code></p>
          <p>Then set the correct IP in: <b>Home → Settings → Server URL</b></p>
          <br>
          <button class="btn" onclick="location.reload()">⟳ RETRY</button>
        </body>
        </html>
    """.trimIndent()

    override fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
        if (keyCode == KeyEvent.KEYCODE_BACK && webView.canGoBack()) {
            webView.goBack()
            return true
        }
        return super.onKeyDown(keyCode, event)
    }

    override fun onResume() { super.onResume(); webView.onResume() }
    override fun onPause()  { super.onPause();  webView.onPause()  }

    override fun onDestroy() {
        webView.stopLoading()
        webView.destroy()
        super.onDestroy()
    }
}
