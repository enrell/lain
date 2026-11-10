package io.github.enrell.lain.download

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import io.github.enrell.lain.R
import io.github.enrell.lain.graph
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch

/**
 * Drains the offline queue (D-248) in a foreground service, so a long
 * copy survives the app going to the background. Each copy resumes from
 * its part file (D-239); stopping the service aborts the transfer and
 * keeps the part.
 */
class DownloadService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var job: Job? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            job?.cancel()
            stopSelf()
            return START_NOT_STICKY
        }
        startInForeground(notification(getString(R.string.download_running), 0, 0))
        if (job?.isActive == true) return START_NOT_STICKY
        val graph = applicationContext.graph
        val tracker = graph.downloads
        val watch = scope.launch {
            tracker.state.collectLatest { s ->
                val (done, total) = s.bytes.values.lastOrNull() ?: (0L to 0L)
                manager().notify(ID, notification(getString(R.string.download_running), done, total))
            }
        }
        job = scope.launch {
            tracker.started()
            val stopped = runCatching { graph.core.offlineRun(tracker).stopped }.getOrElse { it.message ?: it.toString() }
            tracker.finished(stopped)
            watch.cancel()
            stopSelf()
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        applicationContext.graph.downloads.finished(null)
        scope.cancel()
        super.onDestroy()
    }

    private fun manager() = getSystemService(NotificationManager::class.java)

    private fun startInForeground(n: Notification) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(ID, n)
        }
    }

    private fun notification(text: String, done: Long, total: Long): Notification {
        val m = manager()
        if (m.getNotificationChannel(CHANNEL) == null) {
            m.createNotificationChannel(
                NotificationChannel(CHANNEL, getString(R.string.download_channel), NotificationManager.IMPORTANCE_LOW),
            )
        }
        val b = Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setOngoing(true)
        if (total > 0) {
            val pct = ((done * 100) / total).toInt().coerceIn(0, 100)
            b.setProgress(100, pct, false)
        } else {
            b.setProgress(0, 0, true)
        }
        return b.build()
    }

    companion object {
        private const val CHANNEL = "downloads"
        private const val ID = 1
        const val ACTION_STOP = "io.github.enrell.lain.STOP_DOWNLOADS"

        fun start(context: Context) {
            context.startForegroundService(Intent(context, DownloadService::class.java))
        }

        fun stop(context: Context) {
            context.startService(Intent(context, DownloadService::class.java).setAction(ACTION_STOP))
        }
    }
}
