package com.cranibox.android.shell

import android.annotation.SuppressLint
import android.app.ActivityManager
import android.app.ActivityOptions
import android.content.Context
import android.content.Intent
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.os.Build
import android.os.Bundle
import android.os.Process
import android.os.SystemClock
import android.view.Display
import android.view.InputDevice
import android.view.InputEvent
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.Surface
import com.cranibox.android.core.bridge.Bridge
import com.cranibox.android.core.bridge.Keys

/** The shell helper controls exactly one display. Every input path checks its identity. */
// Loaded only by shell app_process, which is exempt from application hidden-API restrictions.
@SuppressLint("PrivateApi", "DiscouragedPrivateApi", "BlockedPrivateApi")
class ShellService(private val context: Context, appUid: Int, private val apk: String) : ShellBinder(appUid) {
    private val userId = appUid / 100000
    private var display: VirtualDisplay? = null
    private var width = 0
    private var height = 0
    private var trusted = false
    @Volatile private var paused = true
    private val activityTasks by lazy {
        Class.forName("android.app.ActivityTaskManager").getMethod("getService").invoke(null)
    }
    private val inputManager by lazy {
        Class.forName("android.hardware.input.InputManager").getDeclaredMethod("getInstance").invoke(null)
    }
    private val setDisplay = InputEvent::class.java.getDeclaredMethod("setDisplayId", Int::class.javaPrimitiveType)

    override fun execute(action: String, args: Bundle, surface: Surface?): Bundle {
        if (action == ShellProtocol.PAUSE) { paused = args.getBoolean("paused", true); return success(true) }
        return synchronized(this) { executeLocked(action, args, surface) }
    }

    private fun executeLocked(action: String, args: Bundle, surface: Surface?): Bundle = when (action) {
        ShellProtocol.IDENTITY -> Bundle().apply { putInt("pid", Process.myPid()); putInt("uid", Process.myUid()); putString("apk", apk) }
        ShellProtocol.DISPLAY -> createDisplay(args, requireNotNull(surface))
        ShellProtocol.STROKE -> success(stroke(args))
        ShellProtocol.KEY -> {
            val code = args.getInt("code", -1)
            val meta = args.getInt("meta")
            val name = args.getString("name") ?: ""
            val allowed = Keys.parse(name)
            require(allowed != null && allowed.keyCode == code && allowed.metaState == meta) { "key not allowed" }
            success(key(code, meta))
        }
        ShellProtocol.TYPE -> {
            val text = args.getString("text") ?: ""
            require(text.isNotEmpty() && text.length <= 16384 && text.all { it == '\n' || it == '\t' || it in ' '..'~' })
            success(typeAscii(text))
        }
        ShellProtocol.TASKS -> Bundle().apply {
            val tasks = tasks().filter { taskDisplay(it) == displayId() }
            putIntArray("ids", tasks.map { it.taskId }.toIntArray())
            putStringArray("packages", tasks.map { it.topActivity?.packageName ?: "" }.toTypedArray())
        }
        ShellProtocol.FOCUS -> {
            check(!paused) { "phone control paused" }
            val id = args.getInt("id", -1)
            require(tasks().any { it.taskId == id && taskDisplay(it) == displayId() })
            // The Recents launch path moves Home forward on the default display.
            // Reorder this existing task without MOVE_TASK_WITH_HOME or a Recents launch.
            val options = ActivityOptions.makeBasic().apply { launchDisplayId = displayId() }
            Class.forName("android.app.IActivityTaskManager").getMethod(
                "moveTaskToFront", Class.forName("android.app.IApplicationThread"), String::class.java,
                Int::class.javaPrimitiveType, Int::class.javaPrimitiveType, Bundle::class.java,
            ).invoke(activityTasks, null, context.opPackageName, id, 0, options.toBundle())
            success(true)
        }
        ShellProtocol.OPEN -> openApp(args.getString("package") ?: "")
        ShellProtocol.RELEASE -> { releaseDisplay(); success(true) }
        ShellProtocol.EXIT -> {
            releaseDisplay()
            Thread { Thread.sleep(100); Process.killProcess(Process.myPid()) }.start()
            success(true)
        }
        else -> throw IllegalArgumentException("unsupported helper action")
    }

    @Synchronized
    fun releaseDisplay() {
        display?.release()
        display = null
    }

    private fun flag(name: String, fallback: Int = 0) = runCatching { DisplayManager::class.java.getField(name).getInt(null) }.getOrDefault(fallback)

    private fun createDisplay(args: Bundle, surface: Surface): Bundle {
        releaseDisplay()
        width = args.getInt("width"); height = args.getInt("height")
        val density = args.getInt("density")
        paused = args.getBoolean("paused", true)
        require(width in 1..4096 && height in 1..4096 && density in 72..1000)
        val manager = context.getSystemService(DisplayManager::class.java)
        val basic = DisplayManager.VIRTUAL_DISPLAY_FLAG_PUBLIC or DisplayManager.VIRTUAL_DISPLAY_FLAG_OWN_CONTENT_ONLY or
            flag("VIRTUAL_DISPLAY_FLAG_SUPPORTS_TOUCH", 64) or flag("VIRTUAL_DISPLAY_FLAG_DESTROY_CONTENT_ON_REMOVAL", 256)
        val privileged = flag("VIRTUAL_DISPLAY_FLAG_TRUSTED", 1024) or
            if (Build.VERSION.SDK_INT >= 34) flag("VIRTUAL_DISPLAY_FLAG_OWN_FOCUS") or flag("VIRTUAL_DISPLAY_FLAG_STEAL_TOP_FOCUS_DISABLED") else 0
        try {
            display = manager.createVirtualDisplay("Cranibox Agent", width, height, density, surface, basic or privileged)
            trusted = true
        } catch (_: SecurityException) {
            display = manager.createVirtualDisplay("Cranibox Agent", width, height, density, surface, basic)
            trusted = false
        }
        check(display != null && displayId() != Display.DEFAULT_DISPLAY)
        return Bundle().apply { putInt("display_id", displayId()); putBoolean("trusted", trusted) }
    }

    private fun displayId(): Int = display?.display?.displayId?.takeIf { it > Display.DEFAULT_DISPLAY }
        ?: error("Agent display is not available")

    private fun inject(event: InputEvent): Boolean {
        val release = (event is KeyEvent && event.action == KeyEvent.ACTION_UP) ||
            (event is MotionEvent && event.action in listOf(MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL))
        if (paused && !release) return false
        setDisplay.invoke(event, displayId())
        return inputManager.javaClass.getMethod("injectInputEvent", InputEvent::class.java, Int::class.javaPrimitiveType)
            .invoke(inputManager, event, 2) as Boolean
    }

    private fun key(code: Int, meta: Int): Boolean {
        val now = SystemClock.uptimeMillis()
        val down = KeyEvent(now, now, KeyEvent.ACTION_DOWN, code, 0, meta, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0, InputDevice.SOURCE_KEYBOARD)
        val accepted = inject(down)
        return inject(KeyEvent.changeAction(down, KeyEvent.ACTION_UP)) && accepted
    }

    private fun typeAscii(text: String): Boolean {
        val events = KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD).getEvents(text.toCharArray()) ?: return false
        val pressed = LinkedHashMap<Int, KeyEvent>()
        try {
            for (event in events) {
                if (paused || !inject(event)) return false
                if (event.action == KeyEvent.ACTION_DOWN) pressed[event.keyCode] = event else pressed.remove(event.keyCode)
            }
            return true
        } finally {
            pressed.values.toList().asReversed().forEach { runCatching { inject(KeyEvent.changeAction(it, KeyEvent.ACTION_UP)) } }
        }
    }

    private fun stroke(args: Bundle): Boolean {
        val xs = args.getIntArray("x") ?: return false
        val ys = args.getIntArray("y") ?: return false
        val ts = args.getLongArray("t") ?: return false
        require(xs.size in 2..256 && ys.size == xs.size && ts.size == xs.size)
        require(xs.all { it in 0 until width } && ys.all { it in 0 until height })
        require(ts.first() == 0L && ts.last() in 1..5000 && (1 until ts.size).all { ts[it - 1] <= ts[it] })
        val start = SystemClock.uptimeMillis()
        var ended = false
        try {
            for (i in xs.indices) {
                val wait = start + ts[i] - SystemClock.uptimeMillis()
                if (wait > 0) SystemClock.sleep(wait)
                if (paused) return false
                val action = when (i) { 0 -> MotionEvent.ACTION_DOWN; xs.lastIndex -> MotionEvent.ACTION_UP; else -> MotionEvent.ACTION_MOVE }
                val event = MotionEvent.obtain(start, SystemClock.uptimeMillis(), action, xs[i].toFloat(), ys[i].toFloat(), 0)
                event.source = InputDevice.SOURCE_TOUCHSCREEN
                try { if (!inject(event)) return false } finally { event.recycle() }
                ended = action == MotionEvent.ACTION_UP
            }
            return true
        } finally {
            if (!ended) {
                val cancel = MotionEvent.obtain(start, SystemClock.uptimeMillis(), MotionEvent.ACTION_CANCEL, xs[0].toFloat(), ys[0].toFloat(), 0)
                cancel.source = InputDevice.SOURCE_TOUCHSCREEN
                try { runCatching { inject(cancel) } } finally { cancel.recycle() }
            }
        }
    }

    @Suppress("UNCHECKED_CAST")
    private fun tasks(): List<ActivityManager.RunningTaskInfo> {
        val method = activityTasks.javaClass.methods.filter { it.name == "getTasks" }
            .firstOrNull { it.parameterTypes.contentEquals(arrayOf(Int::class.javaPrimitiveType, Boolean::class.javaPrimitiveType, Boolean::class.javaPrimitiveType, Int::class.javaPrimitiveType)) }
            ?: activityTasks.javaClass.methods.first { it.name == "getTasks" && it.parameterCount == 3 }
        val args: Array<Any> = if (method.parameterCount == 4) arrayOf(100, false, false, Display.INVALID_DISPLAY) else arrayOf(100, false, false)
        return method.invoke(activityTasks, *args) as List<ActivityManager.RunningTaskInfo>
    }

    private fun openApp(pkg: String): Bundle {
        check(!paused) { "phone control paused" }
        require(Bridge.validPackage(pkg))
        val id = displayId()
        // Fail closed if task enumeration fails: never risk moving a user's foreground app.
        if (tasks().any { taskDisplay(it) == Display.DEFAULT_DISPLAY && taskVisible(it) &&
                (it.topActivity?.packageName == pkg || it.baseActivity?.packageName == pkg) }) {
            return Bundle().apply { putString("result", "in_use_on_main_screen") }
        }
        val intent = context.packageManager.getLaunchIntentForPackage(pkg)
            ?: return Bundle().apply { putString("result", "not_found") }
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_MULTIPLE_TASK)
        val options = ActivityOptions.makeBasic().apply { launchDisplayId = id }
        // An app_process helper is not an ActivityManager-registered application.
        // Like `am start`, supply no ApplicationThread and attribute the call to shell.
        val start = Class.forName("android.app.IActivityTaskManager").getMethod(
            "startActivityAsUser", Class.forName("android.app.IApplicationThread"),
            String::class.java, String::class.java, Intent::class.java, String::class.java,
            android.os.IBinder::class.java, String::class.java, Int::class.javaPrimitiveType,
            Int::class.javaPrimitiveType, Class.forName("android.app.ProfilerInfo"),
            Bundle::class.java, Int::class.javaPrimitiveType,
        )
        val result = start.invoke(activityTasks, null, context.opPackageName, null, intent, null,
            null, null, 0, 0, null, options.toBundle(), userId) as Int
        return Bundle().apply { putString("result", if (result in 0..3) "started" else "error") }
    }

    private fun success(ok: Boolean) = Bundle().apply { putBoolean("ok", ok) }

    private fun taskDisplay(task: ActivityManager.RunningTaskInfo): Int =
        Class.forName("android.app.TaskInfo").getField("displayId").getInt(task)

    private fun taskVisible(task: ActivityManager.RunningTaskInfo): Boolean =
        Class.forName("android.app.TaskInfo").getField("isVisible").getBoolean(task)
}
