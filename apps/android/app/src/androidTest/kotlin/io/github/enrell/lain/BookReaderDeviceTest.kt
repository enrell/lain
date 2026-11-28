package io.github.enrell.lain

import android.graphics.Bitmap
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.enrell.lain.core.CopyState
import io.github.enrell.lain.core.OfflinePolicy
import io.github.enrell.lain.core.bookProgressAt
import io.github.enrell.lain.ui.offline
import io.github.enrell.lain.ui.screens.BOOK_STATUS_TAG
import io.github.enrell.lain.ui.screens.ReaderScreen
import io.github.enrell.lain.ui.screens.readerPageTag
import io.github.enrell.lain.ui.theme.LainTheme
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/**
 * The book readers on a device against a real `lain serve` (the
 * android-device CI job, `scripts/device-test.sh`): the app's own core
 * (the UniFFI bindings) and the reader screens, in two runs of the
 * instrumentation with the server stopped in between.
 *
 * - `phase=online`: log in, read the EPUB (its text in the WebView, a
 *   chapter turn saved as progress on the server), download it, and draw
 *   the PDF's pages.
 * - `phase=offline` (the server is gone): the downloaded EPUB opens from
 *   the device.
 *
 * Arguments (`am instrument -e`): `phase`, `server`, `username`,
 * `password`, `epub` and `pdf` (item ids). Without a `phase` both tests
 * are skipped, so a plain `connectedAndroidTest` passes. Screenshots go
 * to the app's `files/screens/`, which the script pulls with `run-as`.
 */
@RunWith(AndroidJUnit4::class)
class BookReaderDeviceTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val args = InstrumentationRegistry.getArguments()
    private val context = instrumentation.targetContext
    private val graph = context.graph

    private fun arg(name: String): String = args.getString(name)?.takeIf { it.isNotBlank() } ?: throw AssertionError("instrumentation argument '$name' is missing")

    @Test
    fun readOnline() {
        assumeTrue(args.getString("phase") == "online")
        val core = graph.core
        val epub = arg("epub")
        val pdf = arg("pdf")
        // No free-space floor: the emulator's data partition is small.
        core.setOfflinePolicy(OfflinePolicy(maxBytes = 0L, minFreeBytes = 0L, evictWatched = false))
        core.setServer(arg("server"))
        val book = runBlocking {
            core.login(arg("username"), arg("password"))
            core.book(epub)
        }
        assertEquals("reflowable", book.layout)
        assertEquals(2, book.chapters.size)

        var open by mutableStateOf(epub)
        compose.setContent {
            LainTheme(graph.themes.load(graph.prefs.theme)) {
                key(open) { ReaderScreen(core, open, title = if (open == epub) book.title else "Tiny Pages") }
            }
        }

        // The first chapter, laid out in the WebView.
        eventually("the first chapter's text in the WebView") { matches("The first page of a tiny book") > 0 }
        eventually("the status line on chapter 1") { status(context.getString(R.string.book_chapter_n, 1, 2)) }
        shot("epub-online")

        // The next chapter: the place is saved on the server, in virtual pages.
        compose.onNodeWithText(context.getString(R.string.book_next)).performClick()
        // Chapter 1 says "On to the end." too: chapter 2 is the one without its first line.
        eventually("the second chapter's text in the WebView") { matches("The end.") > 0 && matches("The first page of a tiny book") == 0 }
        eventually("the status line on chapter 2") { status(context.getString(R.string.book_chapter_n, 2, 2)) }
        val start = bookProgressAt(book.chapters, 1L, 0.0)
        eventually("the progress saved on the server") {
            val p = runBlocking { core.progress(epub) }
            assertEquals(start.duration, p.durationSec, 1e-6)
            p.positionSec >= start.position - 1e-6
        }
        shot("epub-online-chapter-2")

        // The offline copy, as the app's download does it.
        runBlocking {
            core.offlineAdd(epub)
            val copy = core.offlineFetch(epub, null)
            assertEquals("${copy.code} ${copy.error}", CopyState.DONE, copy.state)
        }
        assertEquals(CopyState.DONE, core.offlineCopy(epub)?.state)

        // The PDF opens in the page reader, its pages drawn from the server.
        compose.runOnUiThread { open = pdf }
        eventually("the PDF's first page drawn", 60_000) {
            compose.onAllNodes(hasTestTag(readerPageTag(0))).fetchSemanticsNodes().isNotEmpty()
        }
        val page = compose.onNodeWithTag(readerPageTag(0)).captureToImage()
        assertTrue("page size ${page.width}x${page.height}", page.width > 0 && page.height > 0)
        eventually("the PDF's status line") {
            compose.onAllNodes(hasText(context.getString(R.string.progress_page, 1, 2), substring = true)).fetchSemanticsNodes().isNotEmpty()
        }
        shot("pdf-online")
    }

    @Test
    fun readOffline() {
        assumeTrue(args.getString("phase") == "offline")
        val core = graph.core
        val epub = arg("epub")
        core.setServer(arg("server"))
        assertNotNull("the session saved by the online run", core.session())
        val down = runCatching { runBlocking { core.health() } }.exceptionOrNull()
        assertNotNull("the server must be stopped for this run", down)
        assertTrue("offline: $down", offline(down!!))
        assertEquals(CopyState.DONE, core.offlineCopy(epub)?.state)

        compose.setContent {
            LainTheme(graph.themes.load(graph.prefs.theme)) { ReaderScreen(core, epub, title = "Tiny Book") }
        }
        // The saved place may or may not be known offline: either chapter will do.
        eventually("a chapter of the offline copy in the WebView") {
            matches("The first page of a tiny book") > 0 || matches("The end.") > 0
        }
        eventually("the status line") { status(context.getString(R.string.book_chapter_n, 1, 2)) || status(context.getString(R.string.book_chapter_n, 2, 2)) }
        shot("epub-offline")
    }

    /** The book reader's status line holds [text]. */
    private fun status(text: String): Boolean =
        compose.onAllNodes(hasTestTag(BOOK_STATUS_TAG) and hasText(text, substring = true)).fetchSemanticsNodes().isNotEmpty()

    /** Waits for [check] to be true (an exception counts as not yet). */
    private fun eventually(what: String, timeoutMs: Long = 30_000, check: () -> Boolean) {
        val end = SystemClock.uptimeMillis() + timeoutMs
        var last: Throwable? = null
        while (SystemClock.uptimeMillis() < end) {
            try {
                if (check()) return
            } catch (e: Throwable) {
                last = e
            }
            Thread.sleep(250)
        }
        shot("timeout-" + what.replace(Regex("[^A-Za-z0-9]+"), "-").trim('-'))
        fail("timed out waiting for $what" + (last?.let { ": $it" } ?: "") + " (WebView: $seen)")
    }

    /** What [matches] last saw of the WebView, for a timeout's message. */
    private var seen = "not looked at"

    /** The reader's WebView, in the activity's views. */
    private fun webView(): WebView? {
        fun find(v: View): WebView? = when (v) {
            is WebView -> v
            is ViewGroup -> (0 until v.childCount).firstNotNullOfOrNull { find(v.getChildAt(it)) }
            else -> null
        }
        val activity = compose.activity
        var found: WebView? = null
        instrumentation.runOnMainSync { found = find(activity.window.decorView) }
        return found
    }

    /**
     * How many times [text] is on the page the WebView shows, by its own
     * find-in-page (the reader runs no script); 0 before it is laid out.
     */
    private fun matches(text: String): Int {
        val web = webView()
        if (web == null) {
            seen = "none in the activity"
            return 0
        }
        val count = AtomicInteger(-1)
        val done = CountDownLatch(1)
        var laidOut = false
        instrumentation.runOnMainSync {
            seen = "${web.width}x${web.height}, content height ${web.contentHeight}, shown ${web.isShown}, url ${web.url?.take(60)}"
            if (web.contentHeight > 0) {
                laidOut = true
                web.setFindListener { _, n, finished ->
                    if (finished) {
                        count.set(n)
                        done.countDown()
                    }
                }
                web.findAllAsync(text)
            }
        }
        if (!laidOut) return 0
        val answered = done.await(5, TimeUnit.SECONDS)
        seen += ", find '$text' " + if (answered) "found ${count.get()}" else "never answered"
        instrumentation.runOnMainSync {
            web.setFindListener(null)
            web.clearMatches()
        }
        return count.get().coerceAtLeast(0)
    }

    /** A screenshot of the device into the app's `files/screens/<name>.png`. */
    private fun shot(name: String) {
        // A frame for the WebView to draw what it laid out.
        Thread.sleep(800)
        val bmp = instrumentation.uiAutomation.takeScreenshot() ?: return
        val dir = File(context.filesDir, "screens").apply { mkdirs() }
        File(dir, "$name.png").outputStream().use { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }
}
