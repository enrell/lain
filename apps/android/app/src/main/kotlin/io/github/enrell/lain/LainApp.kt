package io.github.enrell.lain

import android.app.Application
import android.content.Context
import android.graphics.BitmapFactory
import android.util.LruCache
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import io.github.enrell.lain.core.LainCore
import io.github.enrell.lain.core.OfflinePolicy
import io.github.enrell.lain.download.DownloadTracker
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.withContext
import java.io.File

/** The process-wide objects: one [LainCore], preferences, art and downloads. */
class AppGraph(context: Context) {
    val prefs = Prefs(context.getSharedPreferences("lain", Context.MODE_PRIVATE))

    /** Offline copies live in app-specific external storage when there is some (no permission needed). */
    private val offlineDir: File = context.getExternalFilesDir("offline") ?: File(context.filesDir, "offline")

    val core: LainCore = LainCore(File(context.filesDir, "core").absolutePath, offlineDir.absolutePath).also { core ->
        prefs.offlinePolicy()?.let { runCatching { core.setOfflinePolicy(it) } }
    }

    val themes = ThemeStore(File(context.filesDir, "themes"))
    val downloads = DownloadTracker()
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    private val art = object : LruCache<String, ImageBitmap>(48 * 1024 * 1024) {
        override fun sizeOf(key: String, value: ImageBitmap) = value.width * value.height * 4
    }

    /** An item's art, decoded off the main thread; `null` when there is none. */
    suspend fun art(itemId: String): ImageBitmap? {
        art.get(itemId)?.let { return it }
        val bytes = runCatching { core.thumbnail(itemId) }.getOrNull() ?: return null
        val bmp = withContext(Dispatchers.Default) { decode(bytes, 720) } ?: return null
        art.put(itemId, bmp)
        return bmp
    }

    fun savePolicy(p: OfflinePolicy) {
        core.setOfflinePolicy(p)
        prefs.setOfflinePolicy(p)
    }
}

/** Decodes image bytes, downsampled so the long side is near [maxSide]. */
fun decode(bytes: ByteArray, maxSide: Int): ImageBitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
    var sample = 1
    while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= maxSide) sample *= 2
    val opts = BitmapFactory.Options().apply { inSampleSize = sample }
    return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, opts)?.asImageBitmap()
}

class LainApp : Application() {
    lateinit var graph: AppGraph
        private set

    override fun onCreate() {
        super.onCreate()
        graph = AppGraph(this)
        graph.themes.installBuiltins()
    }
}

val Context.graph: AppGraph get() = (applicationContext as LainApp).graph
