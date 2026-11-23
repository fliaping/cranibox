package com.cranibox.android

import android.content.Context
import android.content.res.ColorStateList
import android.content.res.Configuration
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.ColorDrawable
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.text.InputFilter
import android.text.InputType
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView

/**
 * Material 3 colour roles taken from the system's dynamic (Material You) palette, which
 * every supported Android version (12+) provides.
 */
class Palette(private val context: Context) {
    val dark = context.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES
    private val res = context.resources
    private fun c(id: Int) = res.getColor(id, context.theme)
    private fun pick(light: Int, dark: Int) = c(if (this.dark) dark else light)

    val primary = pick(android.R.color.system_accent1_600, android.R.color.system_accent1_200)
    val onPrimary = if (dark) c(android.R.color.system_accent1_800) else Color.WHITE
    val primaryContainer = pick(android.R.color.system_accent1_100, android.R.color.system_accent1_700)
    val onPrimaryContainer = pick(android.R.color.system_accent1_900, android.R.color.system_accent1_100)
    val secondaryContainer = pick(android.R.color.system_accent2_100, android.R.color.system_accent2_700)
    val onSecondaryContainer = pick(android.R.color.system_accent2_900, android.R.color.system_accent2_100)
    val tertiaryContainer = pick(android.R.color.system_accent3_100, android.R.color.system_accent3_700)
    val onTertiaryContainer = pick(android.R.color.system_accent3_900, android.R.color.system_accent3_100)
    val surface = pick(android.R.color.system_neutral1_10, android.R.color.system_neutral1_900)
    val surfaceContainer = pick(android.R.color.system_neutral1_50, android.R.color.system_neutral1_800)
    val surfaceContainerHigh = pick(android.R.color.system_neutral1_100, android.R.color.system_neutral1_700)
    val onSurface = pick(android.R.color.system_neutral1_900, android.R.color.system_neutral1_100)
    val onSurfaceVariant = pick(android.R.color.system_neutral2_700, android.R.color.system_neutral2_200)
    val outline = pick(android.R.color.system_neutral2_500, android.R.color.system_neutral2_400)
    val outlineVariant = pick(android.R.color.system_neutral2_200, android.R.color.system_neutral2_700)
    val error = if (dark) Color.rgb(0xFF, 0xB4, 0xAB) else Color.rgb(0xBA, 0x1A, 0x1A)
    val errorContainer = if (dark) Color.rgb(0x93, 0x00, 0x0A) else Color.rgb(0xFF, 0xDA, 0xD6)
    val onErrorContainer = if (dark) Color.rgb(0xFF, 0xDA, 0xD6) else Color.rgb(0x41, 0x00, 0x02)
    val good = if (dark) Color.rgb(0x7D, 0xDB, 0x9A) else Color.rgb(0x1B, 0x7F, 0x3E)
    val warn = if (dark) Color.rgb(0xFB, 0xBF, 0x24) else Color.rgb(0xB4, 0x5F, 0x06)
}

enum class Tone { GOOD, BUSY, WARN, OFF }

/** Small view factory so every screen shares one shape and type scale. */
class Ui(val context: Context, val p: Palette) {
    fun dp(value: Int) = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value.toFloat(), context.resources.displayMetrics).toInt()
    fun dpf(value: Int) = dp(value).toFloat()

    fun rounded(color: Int, radius: Int, stroke: Int? = null) = GradientDrawable().apply {
        setColor(color); cornerRadius = dpf(radius); stroke?.let { setStroke(dp(1), it) }
    }

    private fun ripple(content: Drawable?, radius: Int, on: Int) =
        RippleDrawable(ColorStateList.valueOf(Color.argb(40, Color.red(on), Color.green(on), Color.blue(on))), content, rounded(Color.WHITE, radius))

    fun text(value: CharSequence = "", size: Float = 14f, color: Int = p.onSurface, bold: Boolean = false) = TextView(context).apply {
        text = value; textSize = size; setTextColor(color); setLineSpacing(0f, 1.15f)
        if (bold) typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
    }

    fun icon(res: Int, tint: Int, size: Int = 24) = ImageView(context).apply {
        setImageResource(res); imageTintList = ColorStateList.valueOf(tint)
        layoutParams = LinearLayout.LayoutParams(dp(size), dp(size))
    }

    fun column(gap: Int = 0) = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        if (gap > 0) { showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE; dividerDrawable = spacer(gap) }
    }

    fun row(gap: Int = 0) = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
        if (gap > 0) { showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE; dividerDrawable = spacer(gap) }
    }

    private fun spacer(size: Int) = object : ColorDrawable(Color.TRANSPARENT) {
        override fun getIntrinsicWidth() = dp(size)
        override fun getIntrinsicHeight() = dp(size)
    }

    /** A filled card in the style of FlClash's dashboard widgets: icon + title header, then body. */
    fun card(title: String, iconRes: Int, color: Int = p.surfaceContainer, onColor: Int = p.onSurface, body: LinearLayout.() -> Unit): LinearLayout =
        column(12).apply {
            background = rounded(color, 20)
            setPadding(dp(16), dp(16), dp(16), dp(16))
            addView(row(10).apply {
                addView(icon(iconRes, onColor, 20))
                addView(text(title, 15f, onColor, bold = true))
            })
            body()
        }

    enum class Kind { FILLED, TONAL, OUTLINED, TEXT, DANGER }

    fun button(label: String, kind: Kind = Kind.FILLED, iconRes: Int? = null, action: () -> Unit): TextView {
        val (bg, fg) = when (kind) {
            Kind.FILLED -> p.primary to p.onPrimary
            Kind.TONAL -> p.secondaryContainer to p.onSecondaryContainer
            Kind.OUTLINED, Kind.TEXT -> Color.TRANSPARENT to p.primary
            Kind.DANGER -> p.errorContainer to p.onErrorContainer
        }
        return text(label, 14f, fg, bold = true).apply {
            gravity = Gravity.CENTER
            minHeight = dp(48)
            setPadding(dp(if (iconRes != null) 16 else 24), dp(12), dp(24), dp(12))
            val shape = rounded(bg, 22, if (kind == Kind.OUTLINED) p.outline else null)
            background = ripple(shape, 22, fg)
            iconRes?.let {
                val d = context.getDrawable(it)!!.mutate().apply { setTint(fg); setBounds(0, 0, dp(18), dp(18)) }
                setCompoundDrawablesRelative(d, null, null, null); compoundDrawablePadding = dp(8)
            }
            isClickable = true; isFocusable = true
            setOnClickListener { action() }
        }
    }

    /** A Material 3 style filled text field. */
    fun field(label: String, maxLength: Int): EditText =
        EditText(context).apply {
            hint = label; inputType = InputType.TYPE_CLASS_NUMBER; filters = arrayOf(InputFilter.LengthFilter(maxLength))
            textSize = 16f; setTextColor(p.onSurface); setHintTextColor(p.onSurfaceVariant)
            background = rounded(p.surfaceContainerHigh, 12)
            setPadding(dp(16), dp(12), dp(16), dp(12)); isSingleLine = true
        }

    /** Coloured dot used beside status lines. */
    fun dot(tone: Tone) = View(context).apply {
        background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(toneColor(tone)) }
        layoutParams = LinearLayout.LayoutParams(dp(10), dp(10))
    }

    fun toneColor(tone: Tone) = when (tone) {
        Tone.GOOD -> p.good; Tone.BUSY -> p.primary; Tone.WARN -> p.warn; Tone.OFF -> p.outline
    }

    fun chip(label: String, tone: Tone) = row(6).apply {
        background = rounded(p.surfaceContainerHigh, 8)
        setPadding(dp(10), dp(4), dp(12), dp(4))
        addView(dot(tone))
        addView(text(label, 12f, p.onSurfaceVariant, bold = true))
    }

    fun full() = LinearLayout.LayoutParams(-1, -2)
    fun frame(w: Int, h: Int, gravity: Int) = FrameLayout.LayoutParams(w, h, gravity)
}

/** Maps the node's and phone helper's status lines onto a tone for dots and chips. */
object StatusTone {
    fun node(status: String) = when {
        status == "节点运行中" -> Tone.GOOD
        status.endsWith("…") -> Tone.BUSY
        status == "节点已停止" || status == "节点尚未启动" -> Tone.OFF
        else -> Tone.WARN
    }

    fun phone(state: String) = when {
        state.startsWith("手机控制已连接") -> Tone.GOOD
        state.endsWith("…") -> Tone.BUSY
        state == "手机控制未连接" || state.startsWith("已清除") -> Tone.OFF
        else -> Tone.WARN
    }
}
