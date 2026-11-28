package com.cranibox.android.notify

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.app.Notification
import com.cranibox.android.MainActivity
import com.cranibox.android.R
import com.cranibox.android.api.Copy

class ApprovalNotifier(private val context: Context) {
    private val manager = context.getSystemService(NotificationManager::class.java)
    private val seen = linkedSetOf<String>()

    init {
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_approvals), NotificationManager.IMPORTANCE_HIGH)
                .apply { description = context.getString(R.string.channel_approvals_desc) },
        )
    }

    @Synchronized
    fun reset() {
        seen.clear()
        manager.cancel(ID)
    }

    @Synchronized
    fun publish(ids: List<String>, titles: List<String>) {
        val fresh = ids.filter { it !in seen }
        seen.clear(); seen.addAll(ids)
        if (ids.isEmpty()) {
            manager.cancel(ID)
            return
        }
        if (fresh.isEmpty()) return
        val text = when {
            titles.size == 1 -> titles.first()
            else -> context.getString(R.string.approvals_section) + " · " + ids.size
        }
        val open = PendingIntent.getActivity(
            context, 2,
            Intent(context, MainActivity::class.java)
                .setAction(MainActivity.ACTION_APPROVALS)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.approvals_section))
            .setContentText(text)
            .setStyle(Notification.BigTextStyle().bigText(titles.joinToString("\n").ifBlank { text }))
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        manager.notify(ID, notification)
    }

    companion object {
        const val CHANNEL = "cranibox-approvals"
        private const val ID = 2
        fun titleOf(kind: String, capability: String?, operation: String?) =
            operation?.takeIf { it.isNotBlank() } ?: capability?.let(Copy::capabilityName) ?: Copy.approvalTitle(kind)
    }
}
