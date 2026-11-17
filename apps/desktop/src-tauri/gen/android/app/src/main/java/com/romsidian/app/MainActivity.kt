package com.romsidian.app

import android.content.Intent
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import org.json.JSONObject

class MainActivity : TauriActivity() {
  @Volatile
  private var pendingShareJson: String? = null
  private var attachedWebView: WebView? = null

  inner class ShareBridge {
    @JavascriptInterface
    fun consumePendingShare(): String? {
      val current = pendingShareJson
      pendingShareJson = null
      return current
    }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    handleShareIntent(intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    handleShareIntent(intent)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    attachedWebView = webView
    webView.addJavascriptInterface(ShareBridge(), "RomsidianAndroidShare")
    if (pendingShareJson != null) {
      notifyWebViewShare()
    }
  }

  private fun handleShareIntent(intent: Intent?) {
    if (intent?.action != Intent.ACTION_SEND) return
    val mimeType = intent.type ?: return
    if (!mimeType.startsWith("text/")) return

    val sharedText = intent.getStringExtra(Intent.EXTRA_TEXT)?.trim().orEmpty()
    if (sharedText.isEmpty()) return
    val sharedSubject = intent.getStringExtra(Intent.EXTRA_SUBJECT)?.trim().orEmpty()

    pendingShareJson = JSONObject().apply {
      put("text", sharedText)
      put("subject", sharedSubject)
    }.toString()

    notifyWebViewShare()
  }

  private fun notifyWebViewShare() {
    val webView = attachedWebView ?: return
    webView.post {
      webView.evaluateJavascript(
        "window.dispatchEvent(new CustomEvent('romsidian-android-share'));",
        null
      )
    }
  }
}
