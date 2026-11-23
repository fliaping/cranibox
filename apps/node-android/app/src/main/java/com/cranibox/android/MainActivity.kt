package com.cranibox.android

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.content.ComponentName
import android.content.pm.PackageManager
import android.graphics.Typeface
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.Gravity
import android.view.View
import android.view.WindowInsets
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebResourceError
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import android.widget.ProgressBar
import com.cranibox.android.core.helper.HelperHandoff
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collectLatest
import java.io.ByteArrayInputStream

/**
 * Material 3 shell in the spirit of FlClash: an overview dashboard of cards, the node's own
 * interface in a loopback-only WebView, and the phone-control setup, behind a bottom
 * navigation bar. The node card contains its start/stop action.
 */
class MainActivity : Activity() {
    private enum class Tab(val title: String, val icon: Int) {
        OVERVIEW("概览", R.drawable.ic_dashboard), NODE("设备", R.drawable.ic_hub),
        SCREEN("屏幕", R.drawable.ic_screen), PHONE("手机", R.drawable.ic_phone)
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val app get() = node
    private lateinit var ui: Ui
    private val p get() = ui.p

    private lateinit var web: WebView
    private lateinit var content: FrameLayout
    private lateinit var pageTitle: TextView
    private lateinit var fab: TextView
    private val pages = mutableMapOf<Tab, View>()
    private val navItems = mutableMapOf<Tab, Pair<View, TextView>>()
    private var tab = Tab.OVERVIEW
    private var resumed = false

    // Live views on the overview and phone pages.
    private lateinit var nodeDot: View
    private lateinit var nodeStatus: TextView
    private lateinit var phoneChip: LinearLayout
    private lateinit var phoneStatus: TextView
    private lateinit var setupPhoneStatus: TextView
    private lateinit var setupDot: View
    private lateinit var agentCard: LinearLayout
    private lateinit var agentTitle: TextView
    private lateinit var agentBody: TextView
    private lateinit var agentButton: TextView
    private lateinit var screenStatus: TextView
    private lateinit var overlayButton: TextView
    private lateinit var screenStop: TextView
    private lateinit var pairCode: EditText
    private lateinit var pairingPort: EditText
    private lateinit var connectPort: EditText
    private lateinit var setupProgress: ProgressBar
    private lateinit var nodePlaceholder: LinearLayout
    private lateinit var nodeMessage: TextView
    private val setupActions = mutableListOf<View>()
    @Volatile private var loaded: String? = null
    private var webFailed = false
    private var openDevicesAfterLogin = false
    private var agentState: Triple<Boolean, Boolean, Boolean>? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = Ui(this, Palette(this))
        window.setDecorFitsSystemWindows(false)

        val root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setBackgroundColor(p.surface) }
        pageTitle = ui.text("", 24f, p.onSurface).apply { setPadding(ui.dp(20), ui.dp(16), ui.dp(20), ui.dp(12)) }
        root.addView(pageTitle)
        val body = FrameLayout(this)
        content = FrameLayout(this)
        body.addView(content, FrameLayout.LayoutParams(-1, -1))
        fab = fab()
        root.addView(body, LinearLayout.LayoutParams(-1, 0, 1f))
        val nav = navigationBar()
        root.addView(nav)
        root.setOnApplyWindowInsetsListener { _, insets ->
            val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
            val ime = insets.getInsets(WindowInsets.Type.ime())
            root.setPadding(bars.left, bars.top, bars.right, 0)
            nav.setPadding(0, 0, 0, bars.bottom)
            // The keyboard overlays the bottom bar; pad the pages so focused fields stay visible.
            content.setPadding(0, 0, 0, (ime.bottom - nav.height).coerceAtLeast(0))
            insets
        }

        web = WebView(this)
        configureWebView()
        pages[Tab.OVERVIEW] = scroll(overviewPage())
        pages[Tab.NODE] = FrameLayout(this).apply {
            addView(web, FrameLayout.LayoutParams(-1, -1))
            nodePlaceholder = ui.column(16).apply {
                gravity = Gravity.CENTER
                setPadding(ui.dp(32), ui.dp(24), ui.dp(32), ui.dp(24))
                setBackgroundColor(p.surface)
                addView(ui.icon(R.drawable.ic_hub, p.primary, 40))
                nodeMessage = ui.text("正在启动节点…", 18f, p.onSurface).apply { gravity = Gravity.CENTER }
                addView(nodeMessage)
                addView(ui.text("节点就绪后，这里会显示设备与授权管理。", 14f, p.onSurfaceVariant).apply { gravity = Gravity.CENTER })
                addView(ui.button("重新启动", Ui.Kind.TONAL) { service(NodeService.RESTART) })
                addView(ui.button("查看日志", Ui.Kind.TEXT) { showLog() })
            }
            addView(nodePlaceholder, FrameLayout.LayoutParams(-1, -1))
        }
        pages[Tab.PHONE] = scroll(phonePage())
        pages[Tab.SCREEN] = screenPage()
        pages.values.forEach { content.addView(it, FrameLayout.LayoutParams(-1, -1)) }
        setContentView(root)

        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        service()
        select(savedInstanceState?.getString("tab")?.let { runCatching { Tab.valueOf(it) }.getOrNull() } ?: Tab.OVERVIEW)

        scope.launch { app.status.collectLatest { renderNode(it) } }
        scope.launch { app.adb.state.collectLatest { renderPhone(it) } }
        scope.launch { app.adb.busy.collectLatest { busy ->
            setupProgress.visibility = if (busy) View.VISIBLE else View.GONE
            (setupActions + listOf(pairCode, pairingPort, connectPort)).forEach {
                it.isEnabled = !busy; it.alpha = if (busy) .5f else 1f
            }
        } }
        scope.launch { app.loginUrl.collectLatest { url ->
            if (url != null && url != loaded && NodeUrl.isLocal(url)) {
                loaded = url; webFailed = false; openDevicesAfterLogin = true
                nodeMessage.text = "正在加载设备界面…"
                val devices = android.net.Uri.parse(url).buildUpon().fragment("/app/devices").build().toString()
                web.loadUrl(devices, mapOf("Accept-Language" to "zh-CN,zh;q=0.9"))
            }
            if (url == null) {
                loaded = null; nodePlaceholder.visibility = View.VISIBLE; web.loadUrl("about:blank")
            }
        } }
        scope.launch { while (isActive) { renderAgent(app.bridge.paused); delay(1_000) } }
    }

    // ---- Shell ---------------------------------------------------------------------------

    private fun navigationBar() = LinearLayout(this).apply {
        setBackgroundColor(p.surfaceContainer)
        Tab.entries.forEach { t ->
            val indicator = FrameLayout(context).apply {
                addView(ui.icon(t.icon, p.onSurfaceVariant), ui.frame(ui.dp(24), ui.dp(24), Gravity.CENTER))
            }
            val label = ui.text(t.title, 12f, p.onSurfaceVariant, bold = true).apply { gravity = Gravity.CENTER }
            val item = ui.column(4).apply {
                gravity = Gravity.CENTER_HORIZONTAL
                setPadding(0, ui.dp(12), 0, ui.dp(16))
                addView(indicator, LinearLayout.LayoutParams(ui.dp(64), ui.dp(32)))
                addView(label)
                isClickable = true; isFocusable = true; contentDescription = t.title
                setOnClickListener { select(t) }
            }
            navItems[t] = indicator to label
            addView(item, LinearLayout.LayoutParams(0, -2, 1f))
        }
    }

    private fun select(next: Tab) {
        tab = next
        app.agentScreenVisible.value = resumed && next == Tab.SCREEN
        pageTitle.text = when (next) { Tab.OVERVIEW -> "Cranibox"; Tab.NODE -> "设备"; Tab.SCREEN -> "Agent 屏幕"; Tab.PHONE -> "手机控制" }
        pageTitle.typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
        pages.forEach { (t, v) -> v.visibility = if (t == next) View.VISIBLE else View.GONE }
        navItems.forEach { (t, pair) ->
            val (indicator, label) = pair
            val on = t == next
            indicator.background = if (on) ui.rounded(p.secondaryContainer, 16) else null
            (indicator as FrameLayout).getChildAt(0).let { (it as android.widget.ImageView).imageTintList =
                android.content.res.ColorStateList.valueOf(if (on) p.onSecondaryContainer else p.onSurfaceVariant) }
            label.setTextColor(if (on) p.onSurface else p.onSurfaceVariant)
        }
    }

    private fun fab() = ui.text("", 15f, p.onPrimaryContainer, bold = true).apply {
        gravity = Gravity.CENTER
        minHeight = ui.dp(48)
        setPadding(ui.dp(16), ui.dp(12), ui.dp(16), ui.dp(12))
        background = android.graphics.drawable.RippleDrawable(
            android.content.res.ColorStateList.valueOf(android.graphics.Color.argb(40, 0, 0, 0)),
            ui.rounded(p.primaryContainer, 16), null,
        )
        compoundDrawablePadding = ui.dp(12)
        setOnClickListener {
            when (StatusTone.node(app.status.value)) {
                Tone.OFF -> service()
                Tone.WARN -> service(NodeService.RESTART)
                else -> AlertDialog.Builder(this@MainActivity).setTitle("停止节点？")
                    .setMessage("节点与手机控制都会停止，远端设备将无法连接这台手机。")
                    .setPositiveButton("停止") { _, _ -> service(NodeService.STOP) }.setNegativeButton("取消", null).show()
            }
        }
    }

    private fun scroll(page: View) = ScrollView(this).apply {
        isFillViewport = true; clipToPadding = false
        addView(page)
    }

    // ---- Overview ------------------------------------------------------------------------

    private fun overviewPage() = ui.column(12).apply {
        setPadding(ui.dp(16), ui.dp(4), ui.dp(16), ui.dp(24))

        addView(ui.card("节点", R.drawable.ic_hub, p.primaryContainer, p.onPrimaryContainer) {
            addView(ui.row(10).apply {
                nodeDot = ui.dot(Tone.OFF); addView(nodeDot)
                nodeStatus = ui.text("", 18f, p.onPrimaryContainer)
                addView(nodeStatus, LinearLayout.LayoutParams(0, -2, 1f))
            })
            addView(ui.text("设备配对、消息、任务和访问授权在「设备」页管理。命令与终端以 App 自身权限执行。", 13f, p.onPrimaryContainer))
            addView(ui.row(8).apply {
                addView(fab, LinearLayout.LayoutParams(0, -2, 1f))
                addView(ui.button("打开设备", Ui.Kind.TONAL, R.drawable.ic_open) { select(Tab.NODE) },
                    LinearLayout.LayoutParams(0, -2, 1f))
            })
        }, ui.full())

        addView(ui.card("手机控制", R.drawable.ic_phone) {
            phoneChip = ui.chip("", Tone.OFF); addView(phoneChip, LinearLayout.LayoutParams(-2, -2))
            phoneStatus = ui.text("", 14f, p.onSurfaceVariant); addView(phoneStatus)
            addView(ui.row(8).apply {
                addView(ui.button("设置", Ui.Kind.TONAL) { select(Tab.PHONE) })
                addView(setupAction("重新连接", Ui.Kind.TEXT, R.drawable.ic_refresh) { service(NodeService.CONNECT) })
            })
        }, ui.full())

        agentCard = ui.card("Agent", R.drawable.ic_shield) {
            agentTitle = ui.text("", 16f, p.onSurface, bold = true); addView(agentTitle)
            agentBody = ui.text("", 13f, p.onSurfaceVariant); addView(agentBody)
            addView(ui.button("查看 Agent 屏幕", Ui.Kind.TONAL, R.drawable.ic_screen) { select(Tab.SCREEN) }, ui.full())
            agentButton = ui.button("", Ui.Kind.DANGER) {
                service(if (app.bridge.paused) NodeService.RESUME else NodeService.EMERGENCY)
                scope.launch { delay(300); renderAgent(app.bridge.paused) }
            }
            addView(agentButton, LinearLayout.LayoutParams(-2, -2))
        }
        addView(agentCard, ui.full())

        addView(ui.row(12).apply {
            addView(tile("重启节点", R.drawable.ic_refresh) { service(NodeService.RESTART) }, LinearLayout.LayoutParams(0, -2, 1f))
            addView(tile("节点日志", R.drawable.ic_article) { showLog() }, LinearLayout.LayoutParams(0, -2, 1f))
        }, ui.full())

        addView(ui.row(10).apply {
            setPadding(ui.dp(4), ui.dp(4), ui.dp(4), 0)
            gravity = Gravity.TOP
            addView(ui.icon(R.drawable.ic_info, p.onSurfaceVariant, 18))
            addView(ui.text("Agent 使用独立虚拟屏，最长边 1920 像素。部分 App 不支持副屏，安全页面可能黑屏；含中文的输入会覆盖系统剪贴板。", 12f, p.onSurfaceVariant),
                LinearLayout.LayoutParams(0, -2, 1f))
        }, ui.full())
    }

    /** A small square action tile, like FlClash's dashboard quick widgets. */
    private fun tile(label: String, icon: Int, action: () -> Unit) = ui.column(14).apply {
        background = android.graphics.drawable.RippleDrawable(
            android.content.res.ColorStateList.valueOf(android.graphics.Color.argb(30, 0, 0, 0)), ui.rounded(p.surfaceContainer, 20), null)
        setPadding(ui.dp(16), ui.dp(16), ui.dp(16), ui.dp(16))
        addView(FrameLayout(context).apply {
            background = ui.rounded(p.secondaryContainer, 12)
            addView(ui.icon(icon, p.onSecondaryContainer, 20), ui.frame(ui.dp(20), ui.dp(20), Gravity.CENTER))
        }, LinearLayout.LayoutParams(ui.dp(40), ui.dp(40)))
        addView(ui.text(label, 14f, p.onSurface, bold = true))
        isClickable = true; isFocusable = true
        setOnClickListener { action() }
    }

    // ---- Phone setup ---------------------------------------------------------------------

    private fun screenPage() = ui.column(12).apply {
        setPadding(ui.dp(16), ui.dp(4), ui.dp(16), ui.dp(16))
        screenStatus = ui.text("", 14f, p.onSurface, bold = true)
        addView(screenStatus, ui.full())
        addView(ui.text("实时预览 · 点击画面不会操作手机", 12f, p.onSurfaceVariant), ui.full())
        addView(AgentScreenView(this@MainActivity), LinearLayout.LayoutParams(-1, 0, 1f))
        overlayButton = ui.button("开启跨应用悬浮窗", Ui.Kind.TONAL) { toggleOverlay() }
        addView(overlayButton, ui.full())
        addView(ui.row(8).apply {
            addView(ui.button("连接设置", Ui.Kind.TEXT) { select(Tab.PHONE) }, LinearLayout.LayoutParams(0, -2, 1f))
            screenStop = ui.button("停止 Agent", Ui.Kind.DANGER) {
                service(if (app.bridge.paused) NodeService.RESUME else NodeService.EMERGENCY)
            }
            addView(screenStop, LinearLayout.LayoutParams(0, -2, 1f))
        }, ui.full())
    }

    private fun toggleOverlay() {
        if (AgentOverlayService.available.value) {
            AgentOverlayService.setRequested(this, !AgentOverlayService.requested.value)
            renderScreen()
            return
        }
        AlertDialog.Builder(this).setTitle("开启 Agent 屏幕悬浮窗")
            .setMessage("请在下一页启用「Cranibox · Agent 屏幕悬浮窗」。此无障碍服务只展示独立屏幕画面，不读取其他应用内容，也不执行触摸。启用后会显示悬浮窗，可随时关闭。")
            .setPositiveButton("前往开启") { _, _ ->
                AgentOverlayService.setRequested(this, true)
                val details = Intent("android.settings.ACCESSIBILITY_DETAILS_SETTINGS").putExtra(
                    Intent.EXTRA_COMPONENT_NAME, ComponentName(this, AgentOverlayService::class.java))
                runCatching { startActivity(details) }
                    .onFailure { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }
            }.setNegativeButton("取消", null).show()
    }

    private fun renderScreen() {
        val connected = app.phone.connected.value
        screenStatus.text = when {
            !connected -> "手机控制未连接"
            app.bridge.paused -> "Agent 已暂停 · 画面仍可查看"
            else -> "手机控制已连接 · 实时画面"
        }
        screenStop.text = if (app.bridge.paused) "在本机恢复" else "停止 Agent"
        overlayButton.text = when {
            !AgentOverlayService.available.value -> "开启跨应用悬浮窗"
            !AgentOverlayService.requested.value -> "显示悬浮窗"
            !connected -> "取消悬浮窗 · 等待手机连接"
            else -> "关闭悬浮窗"
        }
    }

    private fun phonePage() = ui.column(12).apply {
        setPadding(ui.dp(16), ui.dp(4), ui.dp(16), ui.dp(24))

        addView(ui.row(10).apply {
            background = ui.rounded(p.surfaceContainer, 16)
            setPadding(ui.dp(16), ui.dp(14), ui.dp(16), ui.dp(14))
            setupDot = ui.dot(Tone.OFF); addView(setupDot)
            setupPhoneStatus = ui.text("", 14f, p.onSurface, bold = true)
            addView(setupPhoneStatus, LinearLayout.LayoutParams(0, -2, 1f))
            setupProgress = ProgressBar(context).apply { visibility = View.GONE; contentDescription = "正在设置手机控制" }
            addView(setupProgress, LinearLayout.LayoutParams(ui.dp(24), ui.dp(24)))
        }, ui.full())

        addView(ui.card("无线调试配对", R.drawable.ic_link) {
            addView(steps(
                "在开发者选项中开启「无线调试」",
                "选择「使用配对码配对设备」，保持弹窗打开",
                "从 Cranibox 通知输入六位配对码，或填在下方",
            ))
            addView(ui.button("打开无线调试", Ui.Kind.TONAL, R.drawable.ic_open) {
                service(NodeService.SHOW_PAIR)
                runCatching { startActivity(Intent("android.settings.WIRELESS_DEBUGGING_SETTINGS")) }
                    .onFailure { startActivity(Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS)) }
            }, ui.full())
            pairCode = ui.field("输入六位数字", 6)
            pairingPort = ui.field("自动查找", 5)
            addView(labeledField("配对码", pairCode), ui.full())
            addView(labeledField("配对端口 · 选填", pairingPort), ui.full())
            addView(setupAction("配对并启动", Ui.Kind.FILLED) {
                val code = pairCode.text.toString()
                if (code.length != 6) toast("请输入六位配对码")
                else port(pairingPort) { service(NodeService.PAIR, code, it); pairCode.text.clear() }
            }, ui.full())
        }, ui.full())

        addView(ui.card("重新连接", R.drawable.ic_refresh) {
            addView(ui.text("已配对过的手机可直接连接。端口一般会自动查找，找不到时填写无线调试主页上的端口。", 13f, p.onSurfaceVariant))
            addView(ui.text("配对端口和连接端口是两个不同的端口。连接时请用无线调试主页「IP 地址和端口」中的端口。", 13f, p.onSurfaceVariant))
            connectPort = ui.field("自动查找", 5)
            addView(labeledField("连接端口 · 选填", connectPort), ui.full())
            addView(ui.row(8).apply {
                addView(setupAction("连接", Ui.Kind.TONAL) { port(connectPort) { service(NodeService.CONNECT, port = it) } })
            }, ui.full())
        }, ui.full())

        addView(ui.card("用电脑 USB 启动", R.drawable.ic_terminal) {
            addView(ui.text("没有无线调试时，可连接电脑执行一条命令启动助手。", 13f, p.onSurfaceVariant))
            addView(ui.button("查看命令", Ui.Kind.OUTLINED) { showUsb() }, LinearLayout.LayoutParams(-2, -2))
        }, ui.full())

        addView(ui.card("危险操作", R.drawable.ic_delete, p.surfaceContainer, p.error) {
            addView(ui.row(8).apply {
                addView(setupAction("清除配对", Ui.Kind.TEXT) {
                    AlertDialog.Builder(this@MainActivity).setTitle("清除无线调试配对？")
                        .setMessage("清除 Cranibox 的调试钥匙并停止手机控制，之后需要重新配对。")
                        .setPositiveButton("清除") { _, _ -> service(NodeService.FORGET) }.setNegativeButton("取消", null).show()
                }.apply { setTextColor(p.error) })
                addView(ui.button("停止节点", Ui.Kind.TEXT) { service(NodeService.STOP) }.apply { setTextColor(p.error) })
            })
        }, ui.full())
    }

    private fun setupAction(label: String, kind: Ui.Kind, icon: Int? = null, action: () -> Unit) =
        ui.button(label, kind, icon, action).also { setupActions.add(it) }

    private fun labeledField(label: String, field: EditText) = ui.column(6).apply {
        val caption = ui.text(label, 12f, p.onSurfaceVariant)
        field.id = View.generateViewId(); caption.labelFor = field.id
        addView(caption); addView(field, ui.full())
    }

    private fun steps(vararg lines: String) = ui.column(10).apply {
        lines.forEachIndexed { i, line ->
            addView(ui.row(12).apply {
                addView(ui.text("${i + 1}", 12f, p.onPrimary, bold = true).apply {
                    gravity = Gravity.CENTER; background = ui.rounded(p.primary, 11)
                }, LinearLayout.LayoutParams(ui.dp(22), ui.dp(22)))
                addView(ui.text(line, 14f, p.onSurface), LinearLayout.LayoutParams(0, -2, 1f))
            })
        }
    }

    // ---- Rendering -----------------------------------------------------------------------

    private fun renderNode(status: String) {
        val tone = StatusTone.node(status)
        nodeStatus.text = status
        if (loaded == null) nodeMessage.text = status
        nodeDot.background = ui.dot(tone).background
        val start = tone == Tone.OFF || tone == Tone.WARN
        fab.text = when (tone) { Tone.OFF -> "启动"; Tone.WARN -> "重新启动"; else -> "停止" }
        val icon = getDrawable(if (start) R.drawable.ic_play else R.drawable.ic_stop)!!.mutate().apply {
            setTint(p.onPrimaryContainer); setBounds(0, 0, ui.dp(24), ui.dp(24))
        }
        fab.setCompoundDrawablesRelative(icon, null, null, null)
    }

    private fun renderPhone(state: String) {
        val tone = StatusTone.phone(state)
        val label = when (tone) { Tone.GOOD -> "已连接"; Tone.BUSY -> "处理中"; Tone.WARN -> "需要处理"; Tone.OFF -> "未连接" }
        phoneChip.getChildAt(0).background = ui.dot(tone).background
        (phoneChip.getChildAt(1) as TextView).text = label
        phoneStatus.text = state
        setupPhoneStatus.text = state
        setupDot.background = ui.dot(tone).background
    }

    private fun renderAgent(isPaused: Boolean) {
        renderScreen()
        val state = Triple(isPaused, app.status.value == "节点运行中", app.phone.connected.value)
        if (agentState == state) return
        agentState = state
        if (isPaused) {
            agentCard.background = ui.rounded(p.errorContainer, 20)
            agentTitle.text = "Agent 已暂停"; agentTitle.setTextColor(p.onErrorContainer)
            agentBody.text = "远端 Agent 无法操作这台手机，需在本机恢复。"; agentBody.setTextColor(p.onErrorContainer)
            agentButton.text = "在本机恢复"
            agentButton.background = ui.rounded(p.primary, 22); agentButton.setTextColor(p.onPrimary)
        } else {
            agentCard.background = ui.rounded(p.surfaceContainer, 20)
            agentTitle.text = if (state.second && state.third) "Agent 控制已就绪" else "Agent 控制尚未就绪"
            agentTitle.setTextColor(p.onSurface)
            agentBody.text = when {
                !state.second -> "先启动节点，再连接手机控制。"
                !state.third -> "在「手机」页连接手机控制，完成后仍需节点授权与租约。"
                else -> "仍需节点授权与租约。通知栏也可随时停止。"
            }
            agentBody.setTextColor(p.onSurfaceVariant)
            agentButton.text = "立即停止 Agent"
            agentButton.background = ui.rounded(p.errorContainer, 22); agentButton.setTextColor(p.onErrorContainer)
        }
        recolorCardHeader(agentCard, if (isPaused) p.onErrorContainer else p.onSurface)
    }

    private fun recolorCardHeader(card: LinearLayout, color: Int) {
        val header = card.getChildAt(0) as LinearLayout
        (header.getChildAt(0) as android.widget.ImageView).imageTintList = android.content.res.ColorStateList.valueOf(color)
        (header.getChildAt(1) as TextView).setTextColor(color)
    }

    // ---- Dialogs and actions -------------------------------------------------------------

    private fun showLog() = scope.launch {
        val text = withContext(Dispatchers.IO) {
            runCatching { java.io.File(filesDir, "node.log").readText().takeLast(12_000) }.getOrDefault("").ifBlank { "暂无日志" }
        }
        val view = ui.text(text, 11f, p.onSurface).apply {
            typeface = Typeface.MONOSPACE; setTextIsSelectable(true); setPadding(ui.dp(20), ui.dp(8), ui.dp(20), ui.dp(8))
        }
        val box = ScrollView(this@MainActivity).apply { addView(view) }
        AlertDialog.Builder(this@MainActivity).setTitle("节点日志").setView(box).setPositiveButton("关闭", null).show()
        box.post { box.fullScroll(View.FOCUS_DOWN) }
    }

    private fun showUsb() = scope.launch {
        val script = withContext(Dispatchers.IO) { runCatching { app.adb.writeUsbScript() } }
        script.onSuccess { file ->
            val command = HelperHandoff.usbCommand(file.path)
            val view = ui.column(12).apply {
                setPadding(ui.dp(24), ui.dp(8), ui.dp(24), 0)
                addView(ui.text("手机开启 USB 调试并连接电脑，允许调试授权后，在电脑执行：", 14f, p.onSurfaceVariant))
                addView(ui.text(command, 12f, p.onSurface).apply {
                    typeface = Typeface.MONOSPACE; setTextIsSelectable(true)
                    background = ui.rounded(p.surfaceContainerHigh, 12); setPadding(ui.dp(12), ui.dp(12), ui.dp(12), ui.dp(12))
                })
                addView(ui.text("助手启动后可拔线；手机重启后需重新执行。", 13f, p.onSurfaceVariant))
            }
            AlertDialog.Builder(this@MainActivity).setTitle("用电脑启动手机控制").setView(view)
                .setNeutralButton("复制命令") { _, _ ->
                    getSystemService(android.content.ClipboardManager::class.java)
                        .setPrimaryClip(android.content.ClipData.newPlainText("Cranibox", command))
                }
                .setPositiveButton("知道了", null).show()
        }.onFailure { toast(it.message ?: "无法写入启动脚本") }
    }

    @Suppress("SetJavaScriptEnabled")
    private fun configureWebView() {
        web.settings.apply {
            javaScriptEnabled = true; domStorageEnabled = true
            allowFileAccess = false; allowContentAccess = false
            mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            setSupportMultipleWindows(false)
        }
        web.setBackgroundColor(p.surface)
        CookieManager.getInstance().setAcceptThirdPartyCookies(web, false)
        web.webViewClient = object : WebViewClient() {
            override fun onPageFinished(view: WebView, url: String) {
                if (!webFailed && loaded?.let { NodeUrl.sameOrigin(url, it) } == true) {
                    // The login redirect can discard the initial URL fragment.
                    if (openDevicesAfterLogin) {
                        openDevicesAfterLogin = false
                        view.evaluateJavascript("window.location.replace('#/app/devices');", null)
                    }
                    nodePlaceholder.visibility = View.GONE
                }
            }

            override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
                if (request.isForMainFrame && request.url.toString() != "about:blank") {
                    webFailed = true
                    nodeMessage.text = "设备界面加载失败，请重新启动或查看日志"
                    nodePlaceholder.visibility = View.VISIBLE
                }
            }

            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                val uri = request.url.toString()
                if (loaded?.let { NodeUrl.sameOrigin(uri, it) } == true) return false
                if (request.isForMainFrame && request.hasGesture() && request.url.scheme == "https") {
                    runCatching { startActivity(Intent(Intent.ACTION_VIEW, request.url)) }
                }
                return true
            }

            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? {
                val uri = request.url.toString()
                if (uri == "about:blank" || loaded?.let { NodeUrl.sameOrigin(uri, it) } == true) return null
                return WebResourceResponse("text/plain", "UTF-8", 403, "Forbidden", emptyMap(), ByteArrayInputStream(ByteArray(0)))
            }
        }
    }

    private fun port(field: EditText, action: (Int?) -> Unit) {
        val raw = field.text.toString().trim()
        val port = raw.toIntOrNull()
        if (raw.isNotEmpty() && (port == null || port !in 1..65535)) toast("端口需在 1–65535 之间") else action(port)
    }

    private fun service(action: String? = null, code: String? = null, port: Int? = null) {
        startForegroundService(Intent(this, NodeService::class.java).apply {
            this.action = action
            code?.let { putExtra(NodeService.CODE, it) }; port?.let { putExtra(NodeService.PORT, it) }
        })
    }

    private fun toast(text: String) { Toast.makeText(this, text, Toast.LENGTH_LONG).show() }

    override fun onSaveInstanceState(outState: Bundle) { outState.putString("tab", tab.name); super.onSaveInstanceState(outState) }

    override fun onResume() {
        super.onResume()
        resumed = true
        app.agentScreenVisible.value = tab == Tab.SCREEN
    }

    override fun onPause() {
        resumed = false
        app.agentScreenVisible.value = false
        super.onPause()
    }

    @Deprecated("Deprecated in Java")
    override fun onBackPressed() {
        when {
            tab == Tab.NODE && web.canGoBack() -> web.goBack()
            tab != Tab.OVERVIEW -> select(Tab.OVERVIEW)
            else -> super.onBackPressed()
        }
    }

    override fun onDestroy() { scope.cancel(); content.removeView(web); web.destroy(); super.onDestroy() }
}
