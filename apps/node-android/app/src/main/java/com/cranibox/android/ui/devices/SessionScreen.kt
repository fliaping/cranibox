package com.cranibox.android.ui.devices

import android.annotation.SuppressLint
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.viewinterop.AndroidView
import com.cranibox.android.R
import com.cranibox.android.api.NodeUrl
import com.cranibox.android.node
import java.net.URLEncoder

@OptIn(ExperimentalMaterial3Api::class)
@SuppressLint("SetJavaScriptEnabled")
@Composable
fun SessionScreen(nodeId: String, kind: String, onClose: () -> Unit) {
    val context = LocalContext.current
    val app = context.node
    val loginUrl by app.loginUrl.collectAsState()
    val title = if (kind == "desktop") stringResource(R.string.open_desktop) else stringResource(R.string.open_terminal)
    val hash = if (kind == "desktop") {
        "#/app/device/${URLEncoder.encode(nodeId, "UTF-8")}/desktop"
    } else {
        "#/app/device/${URLEncoder.encode(nodeId, "UTF-8")}"
    }
    val web = remember {
        WebView(context).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            settings.mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            webViewClient = object : WebViewClient() {}
        }
    }
    DisposableEffect(Unit) { onDispose { web.destroy() } }
    BackHandler(onBack = onClose)
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text(title) },
            navigationIcon = {
                IconButton(onClick = onClose) { Icon(Icons.Outlined.Close, contentDescription = stringResource(R.string.session_close)) }
            },
        )
        val url = loginUrl
        if (url != null && NodeUrl.isLocal(url)) {
            val origin = android.net.Uri.parse(url).buildUpon().encodedQuery(null).fragment(null).build().toString().trimEnd('/')
            AndroidView(factory = { web }, modifier = Modifier.fillMaxSize(), update = { view ->
                val target = "$origin/$hash"
                if (view.url != target) view.loadUrl(target)
            })
        } else {
            Text(stringResource(R.string.node_not_ready), modifier = Modifier.padding(20.dp))
        }
    }
}
