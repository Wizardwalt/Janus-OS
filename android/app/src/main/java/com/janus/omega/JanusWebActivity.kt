package com.janus.omega

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.os.Bundle
import android.view.KeyEvent
import android.webkit.*
import android.widget.FrameLayout
import androidx.appcompat.app.AppCompatActivity

class JanusWebActivity : AppCompatActivity() {

    private lateinit var webView: WebView
    private lateinit var prefs: android.content.SharedPreferences

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

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
