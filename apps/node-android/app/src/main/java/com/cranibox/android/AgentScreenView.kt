package com.cranibox.android

import android.content.Context
import android.graphics.Color
import android.view.Gravity
import android.widget.FrameLayout
import android.widget.ImageView
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.combine

/** Read-only screen surface, shared by the full page and accessibility overlay. */
class AgentScreenView(context: Context) : FrameLayout(context) {
    private val ui = Ui(context, Palette(context))
    private val image = ImageView(context).apply {
        scaleType = ImageView.ScaleType.FIT_CENTER
        contentDescription = "Agent 虚拟屏实时画面（只读）"
    }
    private val empty = ui.text(context.getString(R.string.preview_waiting), 14f, Color.WHITE).apply {
        gravity = Gravity.CENTER
        setPadding(ui.dp(20), ui.dp(20), ui.dp(20), ui.dp(20))
    }
    private var collection: Job? = null

    init {
        background = ui.rounded(Color.rgb(20, 23, 28), 16)
        clipToOutline = true
        addView(image, LayoutParams(-1, -1))
        addView(empty, LayoutParams(-1, -1))
    }

    override fun onVisibilityAggregated(isVisible: Boolean) {
        super.onVisibilityAggregated(isVisible)
        collection?.cancel()
        collection = null
        if (isVisible && isAttachedToWindow) {
            collection = CoroutineScope(Dispatchers.Main.immediate).launch {
                combine(context.node.phone.frames, context.node.phone.connected) { frame, connected -> frame to connected }
                    .collect { (frame, connected) ->
                        image.setImageBitmap(frame)
                        empty.visibility = if (frame == null) VISIBLE else GONE
                        empty.text = if (connected) context.getString(R.string.preview_waiting) else context.getString(R.string.preview_disconnected)
                    }
            }
        } else image.setImageDrawable(null)
    }

    override fun onDetachedFromWindow() {
        collection?.cancel(); collection = null
        image.setImageDrawable(null)
        super.onDetachedFromWindow()
    }
}
