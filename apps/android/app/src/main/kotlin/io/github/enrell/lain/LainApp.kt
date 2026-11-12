package io.github.enrell.lain

import androidx.compose.ui.graphics.asAndroidBitmap
import kotlinx.coroutines.async
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
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

    private val art = object : LruCache<String, ImageBitmap>(64 * 1024 * 1024) {
        override fun sizeOf(key: String, value: ImageBitmap) = value.width * value.height * 4
    }
    private val artDir = File(context.cacheDir, "art").apply { mkdirs() }

    /** A few fetches at a time: a fast scroll must not queue a hundred requests ahead of what is on screen. */
    private val artGate = Semaphore(6)

    /** The art already in memory, without suspending: a card that scrolls back paints at once. */
    fun artNow(itemId: String): ImageBitmap? = art.get(itemId)

    /**
     * An item's art: from memory, else from this device's disk (covers
     * rarely change, so a copy is good for a week), else from the server.
     * Decoding is off the main thread and at card size.
     */
    suspend fun art(itemId: String): ImageBitmap? {
        art.get(itemId)?.let { return it }
        val file = File(artDir, itemId.replace(Regex("[^A-Za-z0-9_-]"), "_"))
        val bmp = withContext(Dispatchers.IO) {
            val fresh = file.isFile && System.currentTimeMillis() - file.lastModified() < ART_TTL_MS
            val bytes = if (fresh) {
                runCatching { file.readBytes() }.getOrNull()
            } else {
                artGate.withPermit { runCatching { core.thumbnail(itemId) }.getOrNull() }?.also { runCatching { file.writeBytes(it) } }
                    // Unreachable server: an old copy is better than a blank card.
                    ?: runCatching { file.takeIf { it.isFile }?.readBytes() }.getOrNull()
            }
            bytes?.let { decode(it, 480) }
        } ?: return null
        art.put(itemId, bmp)
        // Cards take the shape of their picture; learned here, off composition.
        io.github.enrell.lain.ui.screens.Shapes.record(itemId, bmp.width, bmp.height)
        return bmp
    }

    private val boards = object : LruCache<String, Board>(2) {}

    /**
     * An item's storyboard, decoded: every preview of the seek bar is a
     * tile cut out of it, so nothing is fetched while dragging. The first
     * call for an item waits while the server makes the picture; `null`
     * when it has none.
     */
    suspend fun storyboard(itemId: String): Board? {
        boards.get(itemId)?.let { return it }
        return scope.async(Dispatchers.IO) {
            val sheet = runCatching { core.storyboard(itemId) }.getOrNull() ?: return@async null
            val bmp = BitmapFactory.decodeByteArray(sheet.image, 0, sheet.image.size)?.asImageBitmap() ?: return@async null
            val columns = sheet.columns.toInt().coerceAtLeast(1)
            val rows = ((sheet.count.toInt() + columns - 1) / columns).coerceAtLeast(1)
            Board(bmp, sheet.intervalSec.coerceAtLeast(1), sheet.count.toInt().coerceAtLeast(1), columns, bmp.width / columns, bmp.height / rows)
                .also { boards.put(itemId, it) }
        }.await()
    }

    private val ambient = object : LruCache<String, ImageBitmap>(64) {}

    /**
     * An item's art as a soft wash of its colours: the picture shrunk to a
     * few pixels, to be drawn large behind a hero. Text printed on a
     * poster disappears in it, so a title set over it stays readable; and
     * it needs no blur support from the system.
     */
    suspend fun ambient(itemId: String): ImageBitmap? {
        ambient.get(itemId)?.let { return it }
        val full = art(itemId) ?: return null
        return withContext(Dispatchers.Default) {
            // Halved step by step, so each pixel is the average of what it stands
            // for (one jump to a few pixels samples points, and lettering survives
            // as bright smears). Three columns keep the art's colours and none of
            // its shapes; drawn large with filtering, they are a smooth wash.
            var cur = full.asAndroidBitmap()
            while (cur.width > 6 && cur.height > 6) {
                cur = android.graphics.Bitmap.createScaledBitmap(cur, (cur.width / 2).coerceAtLeast(3), (cur.height / 2).coerceAtLeast(3), true)
            }
            val h = (3f * cur.height / cur.width).toInt().coerceIn(3, 6)
            android.graphics.Bitmap.createScaledBitmap(cur, 3, h, true).asImageBitmap()
        }.also { ambient.put(itemId, it) }
    }

    /** A frame of an item at a time, for the seek bar; a few stay in memory. */
    private val frames = object : LruCache<String, ImageBitmap>(24) {}

    suspend fun frame(itemId: String, atSec: Long): ImageBitmap? {
        val key = "$itemId@$atSec"
        frames.get(key)?.let { return it }
        // In the app's scope: a drag that moves on must not cancel a frame
        // the server is already cutting; it lands in the cache for when the
        // finger comes back to it.
        return scope.async(Dispatchers.IO) {
            runCatching { core.frame(itemId, atSec.toDouble(), 320u) }.getOrNull()?.let { decode(it, 320) }?.also { frames.put(key, it) }
        }.await()
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

/** A decoded storyboard: tile `i`, row by row, is the frame near `i * intervalSec`. */
class Board(val image: ImageBitmap, val intervalSec: Long, val count: Int, val columns: Int, val tileWidth: Int, val tileHeight: Int) {
    /** The top-left corner of the tile for a time, in pixels. */
    fun tileAt(sec: Double): Pair<Int, Int> {
        val i = (sec / intervalSec).toInt().coerceIn(0, count - 1)
        return (i % columns) * tileWidth to (i / columns) * tileHeight
    }
}

private const val ART_TTL_MS = 7L * 24 * 3600 * 1000

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
