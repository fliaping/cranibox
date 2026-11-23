package com.cranibox.android

import android.accessibilityservice.AccessibilityService
import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.graphics.PixelFormat
import android.graphics.Rect
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowInsets
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine
import kotlin.math.abs
import kotlin.math.hypot

/** Only hosts a preview. Does not retrieve accessibility nodes or inject gestures. */
class AgentOverlayService : AccessibilityService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val manager get() = getSystemService(WindowManager::class.java)
    private lateinit var ui: Ui
    private var root: LinearLayout? = null
    private var params: WindowManager.LayoutParams? = null
    private var expanded = true
    private var title: TextView? = null
    private var statusJob: Job? = null
    private var visibilityJob: Job? = null
    private var position: Pair<Int, Int>? = null
    private var preview: AgentScreenView? = null
    private var preferredWidthDp = 248f

    override fun onServiceConnected() {
        super.onServiceConnected()
        ui = Ui(this, Palette(this))
        preferredWidthDp = getSharedPreferences(PREFS, MODE_PRIVATE).getFloat("width_dp", 248f)
            .takeIf { it.isFinite() && it > 0 } ?: 248f
        available.value = true
        requested.value = getSharedPreferences(PREFS, MODE_PRIVATE).getBoolean("visible", false)
        visibilityJob?.cancel()
        visibilityJob = scope.launch {
            combine(requested, node.phone.connected, node.agentScreenVisible) { wanted, connected, fullScreen ->
                wanted && connected && !fullScreen
            }
                .collect { if (it) showWindow() else removeWindow() }
        }
    }

    private fun usableBounds(): Rect {
        val metrics = manager.currentWindowMetrics
        val bars = metrics.windowInsets.getInsetsIgnoringVisibility(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
        return Rect(bars.left + ui.dp(8), bars.top + ui.dp(8),
            metrics.bounds.width() - bars.right - ui.dp(8), metrics.bounds.height() - bars.bottom - ui.dp(8))
    }

    @SuppressLint("RtlHardcoded") // x/y and raw touch coordinates use the physical left edge in every locale.
    private fun showWindow() {
        if (root != null) return
        val area = usableBounds()
        val width = if (expanded) boundedWidth(preferredWidthDp * resources.displayMetrics.density)
            else minOf(ui.dp(196), area.width())
        val card = ScalingOverlayLayout(this, { factor ->
            if (expanded) resize((params?.width ?: width) * factor)
        }, ::saveSize).apply {
            orientation = LinearLayout.VERTICAL
            background = ui.rounded(ui.p.surfaceContainer, 20, ui.p.outlineVariant)
            elevation = ui.dpf(12)
            setPadding(ui.dp(8), ui.dp(4), ui.dp(8), ui.dp(8))
        }
        val header = ui.row().apply {
            title = ui.text("", 13f, ui.p.onSurface, bold = true).apply {
                gravity = Gravity.CENTER_VERTICAL
                minHeight = ui.dp(48)
                maxLines = 1
                ellipsize = android.text.TextUtils.TruncateAt.END
                setPadding(ui.dp(8), 0, 0, 0)
                contentDescription = "拖动 Agent 悬浮窗，点按展开或收起"
                isClickable = true; isFocusable = true
                setOnClickListener { toggleSize() }
            }
            addView(title, LinearLayout.LayoutParams(0, -2, 1f))
            addView(ui.button("×", Ui.Kind.TEXT) { setRequested(this@AgentOverlayService, false) }.apply {
                contentDescription = "关闭 Agent 悬浮窗"
                textSize = 24f; setPadding(0, 0, 0, 0)
            }, LinearLayout.LayoutParams(ui.dp(48), ui.dp(48)))
        }
        card.addView(header)
        if (expanded) {
            preview = AgentScreenView(this).apply { contentDescription = "Agent 实时预览，双指缩放悬浮窗" }
            card.addView(preview, LinearLayout.LayoutParams(-1, previewHeight(width)))
            card.addView(ui.row().apply {
                addView(ui.button("收起", Ui.Kind.TEXT) { toggleSize() }.apply { setPadding(0, 0, 0, 0) },
                    LinearLayout.LayoutParams(0, ui.dp(48), 1f))
                addView(ui.button("停止", Ui.Kind.DANGER) {
                    startForegroundService(Intent(this@AgentOverlayService, NodeService::class.java).setAction(NodeService.EMERGENCY))
                }.apply { setPadding(0, 0, 0, 0); contentDescription = "停止 Agent" }, LinearLayout.LayoutParams(0, ui.dp(48), 1f))
                addView(ui.button("↘", Ui.Kind.TEXT) {
                    resize(ui.dpf(248)); saveSize()
                }.apply {
                    textSize = 24f; setPadding(0, 0, 0, 0)
                    contentDescription = "拖动缩放窗口，点按恢复默认大小"
                    tooltipText = "拖动缩放 · 点按恢复默认"
                    installResize(this)
                }, LinearLayout.LayoutParams(ui.dp(48), ui.dp(48)))
            })
        }
        val layout = WindowManager.LayoutParams(width, WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
                WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
            PixelFormat.TRANSLUCENT).apply {
            gravity = Gravity.TOP or Gravity.LEFT
            fitInsetsTypes = 0
            x = position?.first ?: (area.right - width)
            y = position?.second ?: (area.top + ui.dp(48))
        }
        root = card; params = layout
        installDrag(requireNotNull(title))
        try {
            manager.addView(card, layout)
            card.post { clampPosition() }
            statusJob = scope.launch {
                while (isActive) {
                    title?.text = if (node.bridge.paused) "Agent 已暂停" else if (expanded) "Agent 屏幕 · 实时" else "Agent 屏幕 · 展开"
                    delay(500)
                }
            }
        } catch (e: RuntimeException) {
            removeWindow()
            setRequested(this, false)
            Toast.makeText(this, "无法显示悬浮窗，请重新开启无障碍服务", Toast.LENGTH_LONG).show()
        }
    }

    private fun toggleSize() {
        expanded = !expanded
        removeWindow(); showWindow()
    }

    private fun screenRatio(): Float = node.phone.status().let { it.height.toFloat() / it.width.coerceAtLeast(1) }
        .coerceIn(0.25f, 4f)

    private fun boundedWidth(requested: Float): Int {
        val area = usableBounds()
        val minimum = minOf(ui.dp(200), area.width()).coerceAtLeast(1)
        // In landscape, keep the controls usable and letterbox the portrait preview if necessary.
        val aspectLimit = ((area.height() - ui.dp(120)).coerceAtLeast(1) / screenRatio() + ui.dp(16)).toInt()
        val maximum = minOf(area.width(), maxOf(minimum, aspectLimit)).coerceAtLeast(minimum)
        return requested.toInt().coerceIn(minimum, maximum)
    }

    private fun previewHeight(width: Int): Int = ((width - ui.dp(16)).coerceAtLeast(1) * screenRatio()).toInt()
        .coerceIn(1, (usableBounds().height() - ui.dp(120)).coerceAtLeast(1))

    private fun resize(requested: Float) {
        if (!expanded || !requested.isFinite()) return
        val card = root ?: return
        val layout = params ?: return
        val width = boundedWidth(requested)
        preferredWidthDp = width / resources.displayMetrics.density
        if (layout.width == width) return
        layout.width = width
        preview?.let { it.layoutParams = (it.layoutParams as LinearLayout.LayoutParams).apply { height = previewHeight(width) } }
        // Measure the new size before clamping; the previous laid-out bounds lag behind a drag.
        card.measure(View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED))
        clampPosition(width, card.measuredHeight)
    }

    private fun saveSize() {
        if (expanded) getSharedPreferences(PREFS, MODE_PRIVATE).edit().putFloat("width_dp", preferredWidthDp).apply()
    }

    private fun installResize(handle: View) {
        var downX = 0f; var downY = 0f; var initialWidth = 0; var dragging = false
        val threshold = ViewConfiguration.get(this).scaledTouchSlop
        handle.setOnTouchListener { view, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    downX = event.rawX; downY = event.rawY
                    initialWidth = params?.width ?: 0; dragging = false
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    val dx = event.rawX - downX; val dy = event.rawY - downY
                    if (abs(dx) > threshold || abs(dy) > threshold) dragging = true
                    if (dragging) {
                        val verticalWidth = dy / screenRatio()
                        resize(initialWidth + if (abs(dx) >= abs(verticalWidth)) dx else verticalWidth)
                    }
                    true
                }
                MotionEvent.ACTION_UP -> { if (dragging) saveSize() else view.performClick(); true }
                MotionEvent.ACTION_CANCEL -> { if (dragging) saveSize(); true }
                else -> false
            }
        }
    }

    private fun clampPosition(width: Int? = null, height: Int? = null) {
        val card = root ?: return
        val layout = params ?: return
        val area = usableBounds()
        layout.x = layout.x.coerceIn(area.left, (area.right - (width ?: card.width)).coerceAtLeast(area.left))
        layout.y = layout.y.coerceIn(area.top, (area.bottom - (height ?: card.height)).coerceAtLeast(area.top))
        position = layout.x to layout.y
        if (card.isAttachedToWindow) manager.updateViewLayout(card, layout)
    }

    private fun installDrag(handle: View) {
        val threshold = ViewConfiguration.get(this).scaledTouchSlop
        var downX = 0f; var downY = 0f; var startX = 0; var startY = 0; var dragged = false
        handle.setOnTouchListener { view, event ->
            val layout = params ?: return@setOnTouchListener false
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    downX = event.rawX; downY = event.rawY; startX = layout.x; startY = layout.y; dragged = false
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    val dx = event.rawX - downX; val dy = event.rawY - downY
                    if (abs(dx) > threshold || abs(dy) > threshold) dragged = true
                    if (dragged) { layout.x = startX + dx.toInt(); layout.y = startY + dy.toInt(); clampPosition() }
                    true
                }
                MotionEvent.ACTION_UP -> { if (!dragged) view.performClick(); true }
                MotionEvent.ACTION_CANCEL -> true
                else -> false
            }
        }
    }

    private fun removeWindow() {
        statusJob?.cancel(); statusJob = null
        root?.let { if (it.isAttachedToWindow) runCatching { manager.removeViewImmediate(it) } }
        root = null; params = null; title = null; preview = null
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (root != null) { removeWindow(); showWindow() }
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit
    override fun onInterrupt() = Unit
    override fun onUnbind(intent: Intent?): Boolean {
        available.value = false
        visibilityJob?.cancel(); visibilityJob = null
        removeWindow()
        return super.onUnbind(intent)
    }
    override fun onDestroy() {
        available.value = false
        removeWindow(); scope.cancel()
        super.onDestroy()
    }

    companion object {
        private const val PREFS = "agent-preview"
        val available = MutableStateFlow(false)
        val requested = MutableStateFlow(false)
        fun setRequested(context: Context, show: Boolean) {
            context.getSharedPreferences(PREFS, MODE_PRIVATE).edit().putBoolean("visible", show).apply()
            requested.value = show
        }
    }
}

/** Intercept the whole multi-touch sequence, cancelling a pressed child when a pinch starts. */
private class ScalingOverlayLayout(context: Context, private val scale: (Float) -> Unit, private val finished: () -> Unit) : LinearLayout(context) {
    private var pinching = false
    private var lastSpan = 0f
    private var scaling = false
    // The platform detector's minimum span can exceed the width of a small overlay.
    // Use touch slop instead, while retaining the whole gesture until every finger is lifted.
    private val slop = ViewConfiguration.get(context).scaledTouchSlop * 2

    init { isClickable = true }

    private fun span(event: MotionEvent, skip: Int = -1): Float {
        val fingers = (0 until event.pointerCount).filter { it != skip }
        if (fingers.size < 2) return 0f
        val a = fingers[0]; val b = fingers[1]
        return hypot(event.getX(a) - event.getX(b), event.getY(a) - event.getY(b))
    }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        if (!pinching && event.pointerCount > 1) {
            pinching = true
            val cancel = MotionEvent.obtain(event).apply { action = MotionEvent.ACTION_CANCEL }
            try { super.dispatchTouchEvent(cancel) } finally { cancel.recycle() }
        }
        if (pinching) {
            when (event.actionMasked) {
                MotionEvent.ACTION_POINTER_DOWN -> { lastSpan = span(event); scaling = false }
                MotionEvent.ACTION_MOVE -> {
                    val current = span(event)
                    if (current > 0 && lastSpan > 0 && (scaling || abs(current - lastSpan) > slop)) {
                        scaling = true
                        scale(current / lastSpan)
                        lastSpan = current
                    }
                }
                MotionEvent.ACTION_POINTER_UP -> {
                    finished()
                    lastSpan = span(event, event.actionIndex); scaling = false
                }
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                    finished(); pinching = false; lastSpan = 0f; scaling = false
                }
            }
            return true
        }
        return super.dispatchTouchEvent(event)
    }
}
