package com.cranibox.android

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.media.ImageReader
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.SystemClock
import android.util.DisplayMetrics
import android.view.Display
import com.cranibox.android.core.bridge.*
import com.cranibox.android.shell.ShellClient
import com.cranibox.android.shell.ShellProtocol
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.ByteArrayOutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class AndroidPhone(private val app: NodeApp) : PhoneControl {
    private val lock = Any()
    private val frameLock = Any()
    private val pauseLock = Any()
    @Volatile private var client: ShellClient? = null
    @Volatile private var inputPaused = false
    private var death: IBinder.DeathRecipient? = null
    @Volatile private var reader: ImageReader? = null
    private var frame: Bitmap? = null
    private var worker: HandlerThread? = null
    private var info = PhoneStatus(false, null, 0, 0, false)
    val connected = MutableStateFlow(false)
    private val previewFrames = MutableStateFlow<Bitmap?>(null)
    // Published bitmaps are immutable snapshots. Never recycle them while a View may draw them.
    val frames = previewFrames.asStateFlow()
    @Volatile var displayWidth: Int = 0
        private set
    @Volatile var displayHeight: Int = 0
        private set

    fun attach(helper: ShellClient) = synchronized(lock) {
        if (client?.binder == helper.binder && connected.value) return@synchronized
        disconnect()
        val metrics = DisplayMetrics()
        @Suppress("DEPRECATION")
        app.getSystemService(DisplayManager::class.java).getDisplay(Display.DEFAULT_DISPLAY).getRealMetrics(metrics)
        // Keep frames and Binder transfers bounded on high-resolution phones.
        val scale = minOf(1.0, 1920.0 / maxOf(metrics.widthPixels, metrics.heightPixels))
        val width = maxOf(1, (metrics.widthPixels * scale).toInt())
        val height = maxOf(1, (metrics.heightPixels * scale).toInt())
        val density = (metrics.densityDpi * scale).toInt().coerceIn(72, 1000)
        val thread = HandlerThread("cranibox-frames").apply { start() }
        worker = thread
        displayWidth = width
        displayHeight = height
        val images = ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, 3)
        reader = images
        val handler = Handler(thread.looper)
        var lastFrameAt = 0L
        var pending = false
        val process = Runnable {
            pending = false
            if (reader !== images) return@Runnable
            lastFrameAt = SystemClock.uptimeMillis()
            val image = runCatching { images.acquireLatestImage() }.getOrNull() ?: return@Runnable
            image.use {
                val plane = it.planes[0]
                val paddedWidth = plane.rowStride / plane.pixelStride
                val padded = Bitmap.createBitmap(paddedWidth, height, Bitmap.Config.ARGB_8888)
                padded.copyPixelsFromBuffer(plane.buffer)
                val crop = Bitmap.createBitmap(padded, 0, 0, width, height)
                if (crop !== padded) padded.recycle()
                synchronized(frameLock) {
                    if (reader === images) {
                        frame?.recycle(); frame = crop
                        val previewScale = minOf(1f, 540f / minOf(width, height))
                        val scaled = Bitmap.createScaledBitmap(crop, (width * previewScale).toInt().coerceAtLeast(1),
                            (height * previewScale).toInt().coerceAtLeast(1), true)
                        // createScaledBitmap may return the source when no resizing is needed.
                        previewFrames.value = if (scaled === crop) crop.copy(Bitmap.Config.ARGB_8888, false) else scaled
                    } else crop.recycle()
                }
            }
        }
        images.setOnImageAvailableListener({
            if (!pending && reader === images) {
                pending = true
                // Delay rather than drop the last frame of a short update (e.g. typed text).
                handler.postDelayed(process, (lastFrameAt + 100L - SystemClock.uptimeMillis()).coerceAtLeast(0))
            }
        }, handler)
        val recipient = IBinder.DeathRecipient { synchronized(lock) { if (client?.binder == helper.binder) disconnect() } }
        try {
            helper.binder.linkToDeath(recipient, 0)
            val result = helper.call(ShellProtocol.DISPLAY, Bundle().apply {
                putInt("width", width); putInt("height", height); putInt("density", density); putBoolean("paused", inputPaused)
            }, images.surface)
            val displayId = result.getInt("display_id", -1)
            check(displayId > Display.DEFAULT_DISPLAY) { "helper did not create an isolated display" }
            synchronized(pauseLock) {
                client = helper
                helper.call(ShellProtocol.PAUSE, Bundle().apply { putBoolean("paused", inputPaused) })
            }
            death = recipient
            info = PhoneStatus(true, displayId, width, height, result.getBoolean("trusted"))
            connected.value = true
        } catch (e: Exception) {
            helper.binder.unlinkToDeath(recipient, 0)
            disconnect()
            throw e
        }
    }

    fun disconnect(stopHelper: Boolean = false) = synchronized(lock) {
        val old = client
        death?.let { runCatching { old?.binder?.unlinkToDeath(it, 0) } }
        runCatching { old?.call(if (stopHelper) ShellProtocol.EXIT else ShellProtocol.RELEASE) }
        client = null
        death = null
        connected.value = false
        info = PhoneStatus(false, null, 0, 0, false)
        val oldReader = reader
        reader = null
        oldReader?.setOnImageAvailableListener(null, null)
        // Close on the frame thread, after callbacks holding an Image have completed.
        worker?.let { Handler(it.looper).post { oldReader?.close() } } ?: oldReader?.close()
        // Drain already-posted callbacks before recycling the final frame.
        worker?.quitSafely(); worker = null
        synchronized(frameLock) { frame?.recycle(); frame = null; previewFrames.value = null }
    }

    override fun status(): PhoneStatus = synchronized(lock) { info }

    override fun setPaused(paused: Boolean) = synchronized(pauseLock) {
        inputPaused = paused
        // Independent of the input lock so an emergency stop interrupts long typing/gestures.
        runCatching { client?.call(ShellProtocol.PAUSE, Bundle().apply { putBoolean("paused", paused) }) }
        Unit
    }

    override fun screenshot(region: IntArray?): Png? = synchronized(frameLock) {
        val latest = frame ?: return@synchronized null
        val crop = if (region == null) latest else Bitmap.createBitmap(latest, region[0], region[1], region[2] - region[0], region[3] - region[1])
        try {
            val out = ByteArrayOutputStream()
            check(crop.compress(Bitmap.CompressFormat.PNG, 100, out))
            Png(out.toByteArray(), crop.width, crop.height)
        } finally { if (crop !== latest) crop.recycle() }
    }

    private fun call(action: String, args: Bundle = Bundle()): Bundle = synchronized(lock) {
        check(connected.value) { "helper disconnected" }
        requireNotNull(client).call(action, args)
    }

    override fun tap(x: Int, y: Int) = stroke(listOf(StrokePoint(x, y, 0), StrokePoint(x, y, 40)))
    override fun stroke(points: List<StrokePoint>) = call(ShellProtocol.STROKE, Bundle().apply {
        putIntArray("x", points.map { it.x }.toIntArray()); putIntArray("y", points.map { it.y }.toIntArray())
        putLongArray("t", points.map { it.tMs }.toLongArray())
    }).getBoolean("ok")

    override fun key(keyCode: Int, metaState: Int): Boolean {
        // Send the parsed name too, so the privileged side independently enforces the whitelist.
        val name = KEY_NAMES.firstOrNull { Keys.parse(it) == KeyChord(keyCode, metaState) } ?: return false
        return call(ShellProtocol.KEY, Bundle().apply { putInt("code", keyCode); putInt("meta", metaState); putString("name", name) }).getBoolean("ok")
    }

    override fun typeAscii(text: String) = call(ShellProtocol.TYPE, Bundle().apply { putString("text", text) }).getBoolean("ok")

    override fun paste(text: String): Boolean {
        // Android restricts clipboard access to a focused app. Fail visibly if the OS refuses it.
        val done = CountDownLatch(1)
        var copied = false
        Handler(app.mainLooper).post {
            try {
                copied = runCatching {
                app.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Cranibox Agent", text))
                }.isSuccess
            } finally { done.countDown() }
        }
        if (!done.await(2, TimeUnit.SECONDS) || !copied) return false
        val chord = requireNotNull(Keys.parse("ctrl+v"))
        return key(chord.keyCode, chord.metaState)
    }

    override fun tasks(): List<PhoneTask> = call(ShellProtocol.TASKS).let { result ->
        val ids = result.getIntArray("ids") ?: return emptyList()
        val packages = result.getStringArray("packages") ?: return emptyList()
        ids.zip(packages).map { (id, pkg) -> PhoneTask(id, pkg) }
    }

    override fun focusTask(id: Int) = call(ShellProtocol.FOCUS, Bundle().apply { putInt("id", id) }).getBoolean("ok")

    @Suppress("DEPRECATION")
    override fun apps(): List<PhoneApp> = app.packageManager.queryIntentActivities(
        Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER), 0,
    ).map { PhoneApp(it.activityInfo.packageName, it.loadLabel(app.packageManager).toString()) }.distinctBy { it.packageName }

    override fun openApp(packageName: String): OpenAppResult {
        val result = call(ShellProtocol.OPEN, Bundle().apply { putString("package", packageName) }).getString("result")
        return OpenAppResult.entries.firstOrNull { it.wire == result } ?: OpenAppResult.ERROR
    }

    companion object {
        private val KEY_NAMES = listOf("back", "escape", "enter", "tab", "backspace", "delete", "up", "down", "left", "right",
            "pageup", "pagedown", "home", "end", "space", "ctrl+a", "ctrl+c", "ctrl+v", "ctrl+x", "ctrl+z")
    }
}
