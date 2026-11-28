package io.github.enrell.lain.ui.screens

import android.annotation.SuppressLint
import android.content.Intent
import android.view.GestureDetector
import android.view.MotionEvent
import android.view.ViewGroup
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import io.github.enrell.lain.R
import io.github.enrell.lain.core.BookInfo
import io.github.enrell.lain.core.BookProgress
import io.github.enrell.lain.core.LainCoreInterface
import io.github.enrell.lain.core.LainException
import io.github.enrell.lain.core.bookPositionOf
import io.github.enrell.lain.core.bookProgressAt
import io.github.enrell.lain.graph
import io.github.enrell.lain.ui.theme.LocalLain
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/** A place in a reflowable book: a chapter (0-based) and how far down it (0..1). */
data class BookSpot(val chapter: Int, val fraction: Double)

/**
 * Where the reader goes next: a chapter, at a fraction or at an anchor in it.
 * Not a data class: two jumps to the same place are two jumps (a retry, or
 * the contents' entry of the chapter being read).
 */
class BookJump(val chapter: Int, val fraction: Double = 0.0, val anchor: String = "")

/**
 * The book reader's state (D-306, D-307): the place being read, chapters
 * fetched (a few kept), and progress saved on the shared virtual pages,
 * a moment after the scrolling stops, and once more on close.
 */
class BookReaderModel(
    private val core: LainCoreInterface,
    val book: BookInfo,
    private val scope: CoroutineScope,
) {
    private val _spot = MutableStateFlow(BookSpot(0, 0.0))
    val spot: StateFlow<BookSpot> = _spot.asStateFlow()
    private val chapters = object : LinkedHashMap<Int, String>(4, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Int, String>?) = size > 3
    }
    private var save: Job? = null
    /** Only a place the reader moved to is saved: closing before the restore must not reset it. */
    private var moved = false

    /** The saved place, or the start. */
    suspend fun restore(): BookSpot {
        val saved = runCatching { core.progress(book.itemId) }.getOrNull()
        val spot = if (saved == null) BookSpot(0, 0.0) else bookPositionOf(book.chapters, saved.positionSec, saved.completed).let {
            BookSpot(it.chapter.toInt().coerceIn(0, (book.chapters.size - 1).coerceAtLeast(0)), it.fraction)
        }
        _spot.value = spot
        return spot
    }

    suspend fun chapter(n: Int): String {
        synchronized(chapters) { chapters[n] }?.let { return it }
        val html = core.bookChapter(book.itemId, n.toLong())
        synchronized(chapters) { chapters[n] = html }
        return html
    }

    fun progressOf(spot: BookSpot): BookProgress = bookProgressAt(book.chapters, spot.chapter.toLong(), spot.fraction)

    /** The reader is now at [chapter], [fraction] of the way down. */
    fun report(chapter: Int, fraction: Double) {
        val spot = BookSpot(chapter, fraction.coerceIn(0.0, 1.0))
        if (spot == _spot.value) return
        _spot.value = spot
        moved = true
        save?.cancel()
        save = scope.launch {
            delay(1500)
            persist(spot)
        }
    }

    private suspend fun persist(spot: BookSpot) {
        val p = progressOf(spot)
        runCatching { core.saveProgress(book.itemId, p.position, p.duration, p.completed) }
    }

    /** The reader is closing: the place is saved. */
    fun close(): Job? {
        if (!moved) return null
        save?.cancel()
        val spot = _spot.value
        return scope.launch { persist(spot) }
    }
}

/** `#rrggbb` of a colour, for CSS. */
internal fun cssHex(argb: Int): String = "#" + (argb and 0xFFFFFF).toString(16).padStart(6, '0')

/** Percent-encodes a fragment (an anchor) for a URL. */
internal fun fragmentEncode(s: String): String = buildString {
    for (b in s.toByteArray(Charsets.UTF_8)) {
        val c = b.toInt() and 0xFF
        if (c in 'A'.code..'Z'.code || c in 'a'.code..'z'.code || c in '0'.code..'9'.code || c == '-'.code || c == '.'.code || c == '_'.code || c == '~'.code) {
            append(c.toChar())
        } else {
            append('%').append(c.toString(16).uppercase().padStart(2, '0'))
        }
    }
}

/**
 * The stylesheet laid over a chapter: the app's colours over the book's,
 * a readable measure and line height, room above and below for the bars.
 * The web reader's scroll layout (`readerCss`), without its paged mode.
 */
internal fun bookCss(background: String, foreground: String, accent: String, dark: Boolean): String = listOf(
    "html { background: $background !important; color-scheme: ${if (dark) "dark" else "light"}; }",
    "body { margin: 0 auto !important; padding: 84px 22px 150px !important; max-width: 40em !important; box-sizing: content-box;" +
        " line-height: 1.6 !important; overflow-wrap: break-word; hyphens: auto; -webkit-hyphens: auto;" +
        " background: $background !important; color: $foreground !important; }",
    "body *:not(img):not(svg):not(svg *) { color: inherit !important; background-color: transparent !important; border-color: currentColor !important; }",
    "a, a * { color: $accent !important; }",
    "img, svg, video { max-width: 100% !important; height: auto; }",
    "::selection { background: rgba(127, 127, 127, 0.35); }",
).joinToString("\n")

/**
 * A chapter with the reader's stylesheet last in its head (so it wins),
 * and, for a jump to an anchor, a refresh to that fragment: with
 * JavaScript off it is how the page scrolls itself there.
 */
internal fun styledChapter(html: String, css: String, anchor: String): String {
    val head = buildString {
        append("<style>").append(css).append("</style>")
        if (anchor.isNotEmpty()) append("<meta http-equiv=\"refresh\" content=\"0;url=#").append(fragmentEncode(anchor)).append("\">")
    }
    val at = html.indexOf("</head>", ignoreCase = true)
    return if (at >= 0) html.substring(0, at) + head + html.substring(at) else head + html
}

/** What the WebView's callbacks share with the screen (not state: they run outside composition). */
private class BookView {
    var view: WebView? = null
    /** The chapter on screen. */
    var chapter = 0
    /** A fraction to scroll to once the chapter is laid out; while set, scrolling is not reported. */
    var pending: Double? = null
    /** Bumped for each load, so a late restore of the previous chapter does nothing. */
    var generation = 0

    fun range(v: WebView): Int {
        @Suppress("DEPRECATION")
        val content = (v.contentHeight * v.scale).roundToInt()
        return (content - v.height).coerceAtLeast(0)
    }

    fun fraction(v: WebView): Double {
        val r = range(v)
        return if (r <= 0) 1.0 else (v.scrollY.toDouble() / r).coerceIn(0.0, 1.0)
    }
}

/**
 * The reader of a book read by chapters (EPUB, MOBI, AZW3; D-306): one
 * chapter at a time in a WebView with JavaScript and local file access
 * off, scrolled downwards, in the app's colours. The chapter comes from
 * the server already sanitized, with its resources carrying the token
 * and its links to other chapters on `lain-chapter://`.
 * Keys: ↓/Space/PageDown and ↑/PageUp scroll (on to the next or back to
 * the previous chapter at the ends), ←/→ change chapter, +/- text size.
 */
@SuppressLint("ClickableViewAccessibility")
@Composable
fun BookReaderScreen(
    core: LainCoreInterface,
    book: BookInfo,
    title: String,
    onBack: () -> Unit,
) {
    val l = LocalLain.current
    val context = LocalContext.current
    val prefs = context.graph.prefs
    val scope = remember(book.itemId) { CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate) }
    val model = remember(book.itemId) { BookReaderModel(core, book, scope) }
    val holder = remember(book.itemId) { BookView() }
    val spot by model.spot.collectAsState()
    val count = book.chapters.size
    // The server as the page's base URL: the chapter's own policy allows 'self' only.
    val base = remember { (core.server() ?: "").trimEnd('/') + "/" }
    var jump by remember(book.itemId) { mutableStateOf<BookJump?>(null) }
    var html by remember(book.itemId) { mutableStateOf<String?>(null) }
    // Which load [html] is: the same chapter loaded again is a new page for the WebView.
    var loadKey by remember(book.itemId) { mutableStateOf(0) }
    var error by remember(book.itemId) { mutableStateOf<Throwable?>(null) }
    var chrome by remember { mutableStateOf(true) }
    var contents by remember { mutableStateOf(false) }
    var zoom by remember { mutableStateOf(prefs.bookTextZoom.coerceIn(60, 250)) }
    val focus = remember { FocusRequester() }
    val css = bookCss(cssHex(l.bg.toArgb()), cssHex(l.text.toArgb()), cssHex(l.accent.toArgb()), l.dark)

    DisposableEffect(book.itemId) {
        onDispose {
            val job = model.close()
            if (job == null) scope.cancel() else job.invokeOnCompletion { scope.cancel() }
        }
    }
    LaunchedEffect(book.itemId) {
        val at = model.restore()
        jump = BookJump(at.chapter, at.fraction)
    }
    LaunchedEffect(jump) {
        val j = jump ?: return@LaunchedEffect
        html = null
        error = null
        try {
            val raw = model.chapter(j.chapter)
            holder.chapter = j.chapter
            holder.pending = if (j.anchor.isEmpty()) j.fraction else null
            holder.generation++
            loadKey = holder.generation
            html = styledChapter(raw, css, j.anchor)
        } catch (e: LainException) {
            error = e
        }
    }
    LaunchedEffect(Unit) { focus.requestFocus() }

    fun go(chapter: Int, fraction: Double = 0.0, anchor: String = "") {
        if (chapter !in 0 until count) return
        jump = BookJump(chapter, fraction, anchor)
    }
    fun step(down: Boolean) {
        val v = holder.view ?: return
        if (html == null) return
        val r = holder.range(v)
        when {
            down && v.scrollY < r -> v.scrollTo(0, (v.scrollY + (v.height * 0.85f).roundToInt()).coerceAtMost(r))
            !down && v.scrollY > 0 -> v.scrollTo(0, (v.scrollY - (v.height * 0.85f).roundToInt()).coerceAtLeast(0))
            down -> go(holder.chapter + 1)
            else -> go(holder.chapter - 1, 1.0)
        }
    }
    fun setZoom(z: Int) {
        val v = holder.view
        val next = z.coerceIn(60, 250)
        if (next == zoom) return
        // Text of another size reflows: keep the place, not the pixel offset.
        if (v != null && html != null) holder.pending = holder.fraction(v)
        zoom = next
        prefs.bookTextZoom = next
    }

    if (count == 0) {
        Centered { Text(stringResource(R.string.reader_empty), color = l.muted) }
        return
    }
    val rtl = book.direction == "rtl"
    Box(
        Modifier
            .fillMaxSize()
            .background(l.bg)
            .safeDrawingPadding()
            .focusRequester(focus)
            .focusable()
            .onPreviewKeyEvent { e ->
                if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                when (e.key) {
                    Key.Spacebar, Key.PageDown, Key.DirectionDown -> step(true)
                    Key.PageUp, Key.DirectionUp -> step(false)
                    Key.DirectionRight -> if (rtl) go(holder.chapter - 1) else go(holder.chapter + 1)
                    Key.DirectionLeft -> if (rtl) go(holder.chapter + 1) else go(holder.chapter - 1)
                    Key.MoveHome -> go(0)
                    Key.MoveEnd -> go(count - 1)
                    Key.Plus, Key.Equals, Key.NumPadAdd -> setZoom(zoom + 10)
                    Key.Minus, Key.NumPadSubtract -> setZoom(zoom - 10)
                    Key.C -> contents = true
                    else -> return@onPreviewKeyEvent false
                }
                true
            },
    ) {
        AndroidView(
            factory = { ctx ->
                WebView(ctx).apply {
                    layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
                    // The page is the server's sanitized HTML: no script, no local files.
                    settings.javaScriptEnabled = false
                    settings.allowFileAccess = false
                    settings.allowContentAccess = false
                    settings.setSupportZoom(false)
                    settings.builtInZoomControls = false
                    settings.setSupportMultipleWindows(false)
                    settings.textZoom = zoom
                    // The keys stay with the reader (the Compose box), not the page's links.
                    isFocusable = false
                    isFocusableInTouchMode = false
                    setBackgroundColor(l.bg.toArgb())
                    webViewClient = object : WebViewClient() {
                        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                            val uri = request.url
                            return when (uri.scheme) {
                                "lain-chapter" -> {
                                    val n = uri.host?.toIntOrNull()
                                    if (n != null) go(n, 0.0, uri.fragment ?: "")
                                    true
                                }
                                "http", "https" -> {
                                    // A jump inside this chapter scrolls the page itself.
                                    if (uri.toString().substringBefore('#') == base) return false
                                    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, uri)) }
                                    true
                                }
                                else -> true
                            }
                        }

                        override fun onPageFinished(view: WebView, url: String?) {
                            val gen = holder.generation
                            // Laid out once the content has a height; images may still grow it.
                            fun settle(tries: Int) {
                                if (gen != holder.generation) return
                                if (view.contentHeight == 0 && tries < 40) {
                                    view.postDelayed({ settle(tries + 1) }, 50)
                                    return
                                }
                                val f = holder.pending
                                if (f != null) {
                                    view.scrollTo(0, (f * holder.range(view)).roundToInt())
                                    holder.pending = null
                                }
                                model.report(holder.chapter, holder.fraction(view))
                            }
                            settle(0)
                        }
                    }
                    setOnScrollChangeListener { v, _, _, _, _ ->
                        if (holder.pending == null) model.report(holder.chapter, holder.fraction(v as WebView))
                    }
                    // A tap (not a scroll, not a link) shows or hides the bars.
                    val taps = GestureDetector(ctx, object : GestureDetector.SimpleOnGestureListener() {
                        override fun onSingleTapConfirmed(e: MotionEvent): Boolean {
                            chrome = !chrome
                            return false
                        }
                    })
                    setOnTouchListener { _, ev ->
                        taps.onTouchEvent(ev)
                        false
                    }
                    holder.view = this
                }
            },
            update = { v ->
                if (v.settings.textZoom != zoom) {
                    v.settings.textZoom = zoom
                    val gen = holder.generation
                    v.postDelayed({
                        val f = holder.pending
                        if (gen == holder.generation && f != null) {
                            v.scrollTo(0, (f * holder.range(v)).roundToInt())
                            holder.pending = null
                        }
                    }, 200)
                }
                val page = html
                if (page != null && v.tag != loadKey) {
                    v.tag = loadKey
                    v.loadDataWithBaseURL(base, page, "text/html", "utf-8", null)
                }
            },
            onRelease = { v ->
                holder.view = null
                v.destroy()
            },
            modifier = Modifier.fillMaxSize(),
        )
        if (html == null) {
            Box(Modifier.fillMaxSize().background(l.bg), contentAlignment = Alignment.Center) {
                if (error != null) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(14.dp)) {
                        ErrorLine(error)
                        WButton(stringResource(R.string.book_retry), { jump = jump?.let { BookJump(it.chapter, it.fraction, it.anchor) } ?: BookJump(spot.chapter, spot.fraction) })
                    }
                } else {
                    CircularProgressIndicator(Modifier.size(36.dp), color = l.muted, strokeWidth = 2.dp)
                }
            }
        }
        val glass = l.sheet.copy(alpha = 0.94f)
        val progress = model.progressOf(spot)
        val pages = progress.duration.toInt().coerceAtLeast(1)
        val page = progress.position.toInt().coerceIn(1, pages)
        AnimatedVisibility(chrome, Modifier.align(Alignment.TopCenter), enter = fadeIn() + slideInVertically { -it / 2 }, exit = fadeOut() + slideOutVertically { -it / 2 }) {
            Row(
                Modifier.padding(10.dp).fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(glass).padding(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconBtn(Ic.ArrowLeft, stringResource(R.string.book_back), onBack, tint = l.text)
                Column(Modifier.weight(1f).padding(horizontal = 6.dp)) {
                    Text(title.ifEmpty { book.title }, color = l.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(
                        stringResource(R.string.progress_page, page, pages) + " · " + stringResource(R.string.book_chapter_n, spot.chapter + 1, count),
                        Modifier.padding(top = 2.dp), color = l.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                }
                SizeBtn("A−", stringResource(R.string.book_text_smaller)) { setZoom(zoom - 10) }
                SizeBtn("A+", stringResource(R.string.book_text_larger)) { setZoom(zoom + 10) }
                IconBtn(Ic.Library, stringResource(R.string.book_contents), { contents = true }, tint = l.text)
            }
        }
        AnimatedVisibility(chrome, Modifier.align(Alignment.BottomCenter), enter = fadeIn() + slideInVertically { it / 2 }, exit = fadeOut() + slideOutVertically { it / 2 }) {
            Row(
                Modifier.padding(start = 10.dp, end = 10.dp, top = 10.dp, bottom = 24.dp).fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(glass).padding(8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                WButton(stringResource(R.string.book_previous), { go(spot.chapter - 1, 1.0) }, Modifier.weight(1f), kind = Btn.Secondary, icon = Ic.ChevronLeft, enabled = spot.chapter > 0)
                WButton(stringResource(R.string.book_next), { go(spot.chapter + 1) }, Modifier.weight(1f), kind = Btn.Secondary, icon = Ic.ChevronRight, enabled = spot.chapter < count - 1)
            }
        }
    }
    if (contents) {
        Sheet(stringResource(R.string.book_contents), { contents = false }) {
            val entries = book.toc.filter { it.chapter >= 0 && it.chapter < count }
            if (entries.isEmpty()) {
                Text(stringResource(R.string.book_no_contents), Modifier.padding(vertical = 12.dp), color = l.muted, fontSize = 14.sp)
            }
            for (t in entries) {
                val here = t.chapter.toInt() == spot.chapter
                Text(
                    t.title.ifBlank { stringResource(R.string.book_chapter_n, t.chapter.toInt() + 1, count) },
                    Modifier
                        .fillMaxWidth()
                        .heightIn(min = 44.dp)
                        .clip(RoundedCornerShape(8.dp))
                        .clickable {
                            contents = false
                            go(t.chapter.toInt(), 0.0, t.anchor)
                        }
                        .padding(start = (12 + 16 * t.depth.toInt().coerceIn(0, 4)).dp, end = 12.dp, top = 12.dp, bottom = 12.dp),
                    color = if (here) l.accent else l.text,
                    fontSize = 15.sp,
                    fontWeight = if (here) FontWeight.SemiBold else FontWeight.Normal,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

/** A small text button of the bar (the text size). */
@Composable
private fun SizeBtn(text: String, label: String, onClick: () -> Unit) {
    val l = LocalLain.current
    Box(
        Modifier.size(40.dp).clip(l.buttonShape).clickable(onClickLabel = label, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { Text(text, color = l.text, fontSize = 13.sp, fontWeight = FontWeight.SemiBold) }
}
