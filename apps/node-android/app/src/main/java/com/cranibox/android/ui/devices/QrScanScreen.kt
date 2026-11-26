package com.cranibox.android.ui.devices

import android.Manifest
import android.app.Application
import android.content.pm.PackageManager
import android.util.Size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import com.cranibox.android.NodeApp
import com.cranibox.android.R
import com.cranibox.android.api.PairAccepted
import com.cranibox.android.api.PairLinks
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.launch


class QrViewModel(application: Application) : AndroidViewModel(application) {
    val repo = (application as NodeApp).repository
    fun accept(link: String, onDone: (Result<PairAccepted>) -> Unit) =
        viewModelScope.launch { onDone(runCatching { repo.client.pairAccept(link) }) }
    fun confirm(flow: String, matches: Boolean, onDone: (String?) -> Unit) =
        viewModelScope.launch { onDone(runCatching { repo.client.pairConfirm(flow, matches); repo.refresh() }.exceptionOrNull()?.message) }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun QrScanScreen(
    initialLink: String,
    onBack: () -> Unit,
    snackbar: SnackbarHostState,
    model: QrViewModel = viewModel(),
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var pasted by remember { mutableStateOf(initialLink) }
    var accepted by remember { mutableStateOf<PairAccepted?>(null) }
    fun submit(raw: String) {
        val link = PairLinks.parse(raw)
        if (link == null) {
            scope.launch { snackbar.showSnackbar("请粘贴有效的 cranibox:// 配对链接") }
            return
        }
        model.accept(link) { result ->
            result.onSuccess { accepted = it }
                .onFailure { scope.launch { snackbar.showSnackbar(it.message ?: "配对失败") } }
        }
    }
    LaunchedEffect(initialLink) {
        if (PairLinks.parse(initialLink) != null) submit(initialLink)
    }
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text(stringResource(R.string.scan_title)) },
            navigationIcon = {
                IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = "返回") }
            },
        )
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.scan_hint), color = MaterialTheme.colorScheme.onSurfaceVariant)
            CameraPane(onBarcode = { value -> if (accepted == null) submit(value) })
            OutlinedTextField(
                value = pasted,
                onValueChange = { pasted = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text(stringResource(R.string.scan_paste_hint)) },
                singleLine = false,
            )
            Button(onClick = { submit(pasted) }, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.scan_use_link))
            }
        }
    }
    accepted?.let { result ->
        AlertDialog(
            onDismissRequest = { accepted = null; onBack() },
            title = { Text("确认配对码") },
            text = { Text("请确认两边显示的短码一致：${result.sas}") },
            confirmButton = {
                TextButton(onClick = {
                    model.confirm(result.flowId, true) { err ->
                        if (err == null) { accepted = null; onBack() }
                        err?.let { scope.launch { snackbar.showSnackbar(it) } }
                    }
                }) { Text(stringResource(R.string.codes_match)) }
            },
            dismissButton = {
                TextButton(onClick = {
                    model.confirm(result.flowId, false) { accepted = null; onBack() }
                }) { Text(stringResource(R.string.deny)) }
            },
        )
    }
}

@Composable
private fun CameraPane(onBarcode: (String) -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    var granted by remember {
        mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
    }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
    if (!granted) {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(vertical = 12.dp)) {
            Text(stringResource(R.string.camera_permission), color = MaterialTheme.colorScheme.onSurfaceVariant)
            Button(onClick = { launcher.launch(Manifest.permission.CAMERA) }) { Text(stringResource(R.string.camera_grant)) }
        }
        return
    }
    val handled = remember { AtomicBoolean(false) }
    AndroidView(
        modifier = Modifier.fillMaxWidth().aspectRatio(3f / 4f),
        factory = { ctx ->
            PreviewView(ctx).apply {
                scaleType = PreviewView.ScaleType.FILL_CENTER
                val scanner = BarcodeScanning.getClient()
                val executor = Executors.newSingleThreadExecutor()
                val providerFuture = ProcessCameraProvider.getInstance(ctx)
                providerFuture.addListener({
                    val provider = providerFuture.get()
                    val preview = Preview.Builder().build().also { it.surfaceProvider = this.surfaceProvider }
                    val analysis = ImageAnalysis.Builder()
                        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                        .setResolutionSelector(
                            ResolutionSelector.Builder()
                                .setResolutionStrategy(ResolutionStrategy(Size(1280, 720), ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER))
                                .build(),
                        )
                        .build()
                    analysis.setAnalyzer(executor) { image ->
                        val media = image.image
                        if (media == null) { image.close(); return@setAnalyzer }
                        val input = InputImage.fromMediaImage(media, image.imageInfo.rotationDegrees)
                        scanner.process(input)
                            .addOnSuccessListener { codes ->
                                val value = codes.firstOrNull { it.format == Barcode.FORMAT_QR_CODE }?.rawValue
                                if (!value.isNullOrBlank() && handled.compareAndSet(false, true)) onBarcode(value)
                            }
                            .addOnCompleteListener { image.close() }
                    }
                    runCatching {
                        provider.unbindAll()
                        provider.bindToLifecycle(lifecycle, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
                    }
                }, ContextCompat.getMainExecutor(ctx))
            }
        },
    )
    DisposableEffect(Unit) { onDispose { handled.set(true) } }
}
