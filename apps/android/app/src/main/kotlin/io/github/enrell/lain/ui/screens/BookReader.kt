package io.github.enrell.lain.ui.screens

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.view.GestureDetector
import android.view.MotionEvent
import android.view.ViewConfiguration
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
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import io.github.enrell.lain.R
import io.github.enrell.lain.core.BookInfo
import io.github.enrell.lain.core.BookProgress
import io.github.enrell.lain.core.LainCoreInterface
import io.github.enrell.lain.core.LainException
import io.github.enrell.lain.core.bookAnchorFractionIn
import io.github.enrell.lain.core.bookPagedCss
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
import kotlinx.coroutines.withContext
import kotlin.math.abs
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

    /**
     * How far into chapter [n] (0..1, by its text) the element [anchor]
     * starts: worked out by the core on the chapter already fetched, so a
     * jump needs no script and no second request. A failure is the
     * chapter's start: the jump still happens.
     */
    suspend fun anchorFraction(n: Int, anchor: String): Double {
        if (anchor.isEmpty()) return 0.0
        val html = synchronized(chapters) { chapters[n] }
        return runCatching {
            if (html != null) {
                withContext(Dispatchers.Default) { bookAnchorFractionIn(html, anchor) }
            } else {
                core.bookAnchorFraction(book.itemId, n.toLong(), anchor)
            }
        }.getOrDefault(0.0)
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

/**
 * The stylesheet laid over a chapter: the app's colours over the book's
 * and a readable line height; then either the scroll layout (a readable
 * measure, room above and below for the bars) or, given [paged] (the
 * core's `bookPagedCss` for this view), the paged one. The web reader's
 * `readerCss`.
 */
internal fun bookCss(background: String, foreground: String, accent: String, dark: Boolean, paged: String? = null): String = listOfNotNull(
    "html { background: $background !important; color-scheme: ${if (dark) "dark" else "light"}; }",
    "body { line-height: 1.6 !important; overflow-wrap: break-word; hyphens: auto; -webkit-hyphens: auto;" +
        " background: $background !important; color: $foreground !important; }",
    "body *:not(img):not(svg):not(svg *) { color: inherit !important; background-color: transparent !important; border-color: currentColor !important; }",
    "a, a * { color: $accent !important; }",
    if (paged == null) "body { margin: 0 auto !important; padding: 84px 22px 150px !important; max-width: 40em !important; box-sizing: content-box; }" else null,
    if (paged == null) "img, svg, video { max-width: 100% !important; height: auto; }" else null,
    paged,
    "::selection { background: rgba(127, 127, 127, 0.35); }",
).joinToString("\n")

/** A chapter with the reader's stylesheet last in its head, so it wins. */
internal fun styledChapter(html: String, css: String): String {
    val head = "<style>$css</style>"
    val at = html.indexOf("</head>", ignoreCase = true)
    return if (at >= 0) html.substring(0, at) + head + html.substring(at) else head + html
}

/**
 * The reader's WebView. Its horizontal scroll range, which WebView keeps
 * protected, is what gives the paged layout its page count.
 */
@SuppressLint("ViewConstructor")
private class ReaderWebView(context: Context) : WebView(context) {
    fun horizontalRange(): Int = computeHorizontalScrollRange()
}

/** What the WebView's callbacks share with the screen (not state: they run outside composition). */
private class BookView {
    var view: ReaderWebView? = null
    /** The chapter on screen. */
    var chapter = 0
    /** Where to go once the chapter is laid out; while set, scrolling is not reported. */
    var pending: Double? = null
    /** [pending] is a share of the chapter's text (an anchor's place), not a reading fraction. */
    var pendingText = false
    /** Bumped for each load, so a late restore of the previous page does nothing. */
    var generation = 0
    /** The page on screen is laid out and placed: scrolls are the reader's own. */
    var loaded = false
    /** The page on screen is in the paged layout. */
    var paged = false
    var density = 1f

    @Suppress("DEPRECATION")
    fun content(v: WebView): Int = (v.contentHeight * v.scale).roundToInt()

    fun range(v: WebView): Int = (content(v) - v.height).coerceAtLeast(0)

    fun stride(v: WebView): Double = pageStride(v.width, density)

    fun pages(v: ReaderWebView): Int = pagesIn(v.horizontalRange(), stride(v))

    fun page(v: ReaderWebView): Int = pageOfScroll(v.scrollX, stride(v), pages(v))

    /**
     * Where page [p] starts, as far as the WebView scrolls: the last
     * column may end a margin short of a whole page.
     */
    fun pageX(v: ReaderWebView, p: Int): Int {
        val max = (v.horizontalRange() - v.width).coerceAtLeast(0)
        return pageScrollX(p.coerceIn(0, pages(v) - 1), stride(v)).coerceAtMost(max)
    }

    fun showPage(v: ReaderWebView, p: Int) = v.scrollTo(pageX(v, p), 0)

    fun fraction(v: ReaderWebView): Double {
        if (paged) return pageFraction(page(v), pages(v))
        val r = range(v)
        return if (r <= 0) 1.0 else (v.scrollY.toDouble() / r).coerceIn(0.0, 1.0)
    }

    /** Goes to [pending], if any. */
    fun settle(v: ReaderWebView) {
        val f = pending ?: return
        if (paged) {
            val n = pages(v)
            showPage(v, if (pendingText) pageOfText(f, n) else pageAtFraction(f, n))
        } else {
            val r = range(v)
            // An anchor a little below the top, so the line before it shows it is the place.
            val y = if (pendingText) f * content(v) - v.height * 0.1 else f * r
            v.scrollTo(0, y.roundToInt().coerceIn(0, r))
        }
        pending = null
        pendingText = false
    }
}

/**
 * The reader of a book read by chapters (EPUB, MOBI, AZW3; D-306): one
 * chapter at a time in a WebView with JavaScript and local file access
 * off, in the app's colours, either in pages (the default: CSS columns
 * one view wide, turned by scrolling the WebView sideways) or scrolled
 * downwards; the choice is kept. The chapter comes from the core already
 * sanitized, with its resources carrying the token (or inlined, offline)
 * and every link to an anchor, in this chapter or another, on
 * `lain-chapter://`, so the reader places it itself, by the anchor's
 * share of the chapter's text.
 * Keys: →/↓/Space/PageDown/volume down and ←/↑/PageUp/volume up turn
 * pages (or scroll), on to the next or back to the previous chapter at
 * the ends; [ and ] change chapter, +/- text size, L the layout, C the
 * contents. Taps on the left and right thirds and swipes turn pages.
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
    val density = LocalDensity.current.density
    val scope = remember(book.itemId) { CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate) }
    val model = remember(book.itemId) { BookReaderModel(core, book, scope) }
    val holder = remember(book.itemId) { BookView() }
    val spot by model.spot.collectAsState()
    val count = book.chapters.size
    // The server as the page's base URL: the chapter's own policy allows 'self' only.
    val base = remember { (core.server() ?: "").trimEnd('/') + "/" }
    var jump by remember(book.itemId) { mutableStateOf<BookJump?>(null) }
    // The chapter as the core gave it; the page shown is it with the reader's style.
    var raw by remember(book.itemId) { mutableStateOf<String?>(null) }
    // Which chapter load [raw] is: the same chapter loaded again is a new page for the WebView.
    var loadKey by remember(book.itemId) { mutableStateOf(0) }
    var error by remember(book.itemId) { mutableStateOf<Throwable?>(null) }
    var chrome by remember { mutableStateOf(true) }
    var contents by remember { mutableStateOf(false) }
    var zoom by remember { mutableStateOf(prefs.bookTextZoom.coerceIn(60, 250)) }
    var paged by remember { mutableStateOf(prefs.bookPaged) }
    var viewport by remember { mutableStateOf(IntSize.Zero) }
    val focus = remember { FocusRequester() }
    // The paged layout is sized to the view, so it waits for the view's size.
    val pagedCss = remember(paged, viewport, zoom, density) {
        if (paged && viewport.width > 0 && viewport.height > 0) {
            bookPagedCss(pageCssWidth(viewport.width, density), viewport.height / density.toDouble(), zoom.toLong())
        } else {
            null
        }
    }
    val css = bookCss(cssHex(l.bg.toArgb()), cssHex(l.text.toArgb()), cssHex(l.accent.toArgb()), l.dark, pagedCss)
    val ready = !paged || pagedCss != null
    val html = remember(raw, css, ready) { if (ready) raw?.let { styledChapter(it, css) } else null }

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
        val v = holder.view
        if (j.anchor.isNotEmpty() && raw != null && holder.loaded && j.chapter == holder.chapter && v != null) {
            // An anchor in the chapter on screen (a note, a contents entry): no reload.
            val f = model.anchorFraction(j.chapter, j.anchor)
            if (holder.loaded && holder.chapter == j.chapter) {
                holder.pending = f
                holder.pendingText = true
                holder.settle(v)
                model.report(holder.chapter, holder.fraction(v))
            }
            return@LaunchedEffect
        }
        raw = null
        error = null
        holder.loaded = false
        try {
            val chapterHtml = model.chapter(j.chapter)
            val f = if (j.anchor.isEmpty()) j.fraction else model.anchorFraction(j.chapter, j.anchor)
            holder.chapter = j.chapter
            holder.pending = f
            holder.pendingText = j.anchor.isNotEmpty()
            loadKey++
            raw = chapterHtml
        } catch (e: LainException) {
            error = e
        }
    }
    LaunchedEffect(Unit) { focus.requestFocus() }

    fun go(chapter: Int, fraction: Double = 0.0, anchor: String = "") {
        if (chapter !in 0 until count) return
        jump = BookJump(chapter, fraction, anchor)
    }
    // These two run from the WebView's listeners too: they read the holder, not composition values.
    fun step(down: Boolean) {
        val v = holder.view ?: return
        if (!holder.loaded) return
        val r = holder.range(v)
        when {
            down && v.scrollY < r -> v.scrollTo(0, (v.scrollY + (v.height * 0.85f).roundToInt()).coerceAtMost(r))
            !down && v.scrollY > 0 -> v.scrollTo(0, (v.scrollY - (v.height * 0.85f).roundToInt()).coerceAtLeast(0))
            down -> go(holder.chapter + 1)
            else -> go(holder.chapter - 1, 1.0)
        }
    }
    fun turn(forward: Boolean) {
        val v = holder.view ?: return
        if (!holder.loaded) return
        if (!holder.paged) return step(forward)
        chrome = false
        val pages = holder.pages(v)
        val p = holder.page(v)
        when {
            forward && p < pages - 1 -> holder.showPage(v, p + 1)
            !forward && p > 0 -> holder.showPage(v, p - 1)
            forward -> go(holder.chapter + 1)
            else -> go(holder.chapter - 1, 1.0)
        }
    }
    fun setZoom(z: Int) {
        val v = holder.view
        val next = z.coerceIn(60, 250)
        if (next == zoom) return
        // Text of another size reflows: keep the place, not the pixel offset.
        if (v != null && holder.loaded && holder.pending == null) holder.pending = holder.fraction(v)
        zoom = next
        prefs.bookTextZoom = next
    }
    fun setLayout(p: Boolean) {
        if (p == paged) return
        // The page is loaded again in the other layout, at the same place (see `update`).
        paged = p
        prefs.bookPaged = p
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
                    Key.Spacebar, Key.PageDown, Key.DirectionDown, Key.VolumeDown -> turn(true)
                    Key.PageUp, Key.DirectionUp, Key.VolumeUp -> turn(false)
                    // Pages run left to right on screen; scrolled, ←/→ change chapter in the book's direction.
                    Key.DirectionRight -> if (paged) turn(true) else if (rtl) go(holder.chapter - 1) else go(holder.chapter + 1)
                    Key.DirectionLeft -> if (paged) turn(false) else if (rtl) go(holder.chapter + 1) else go(holder.chapter - 1)
                    Key.LeftBracket -> go(holder.chapter - 1)
                    Key.RightBracket -> go(holder.chapter + 1)
                    Key.MoveHome -> go(0)
                    Key.MoveEnd -> go(count - 1)
                    Key.Plus, Key.Equals, Key.NumPadAdd -> setZoom(zoom + 10)
                    Key.Minus, Key.NumPadSubtract -> setZoom(zoom - 10)
                    Key.L -> setLayout(!paged)
                    Key.C -> contents = true
                    else -> return@onPreviewKeyEvent false
                }
                true
            },
    ) {
        AndroidView(
            factory = { ctx ->
                ReaderWebView(ctx).apply {
                    val web = this
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
                    // Pages are turned, not dragged: no scroll bar flashing along the bottom.
                    isHorizontalScrollBarEnabled = false
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
                            val v = view as? ReaderWebView ?: return
                            val gen = holder.generation
                            // Laid out once the content has a height (and, paged, a width
                            // that holds still: the scroll range arrives a frame or two late).
                            fun settle(tries: Int, lastRange: Int) {
                                if (gen != holder.generation) return
                                val range = v.horizontalRange()
                                val waiting = v.contentHeight == 0 || (holder.paged && range != lastRange)
                                if (waiting && tries < 40) {
                                    v.postDelayed({ settle(tries + 1, range) }, 50)
                                    return
                                }
                                holder.settle(v)
                                holder.loaded = true
                                model.report(holder.chapter, holder.fraction(v))
                            }
                            settle(0, -1)
                        }
                    }
                    setOnScrollChangeListener { v, _, _, _, _ ->
                        val w = v as ReaderWebView
                        if (holder.pending != null || !holder.loaded) return@setOnScrollChangeListener
                        if (holder.paged) {
                            // Only page starts: a scroll the page made itself (a selection) snaps to one.
                            val want = holder.pageX(w, holder.page(w))
                            if (abs(w.scrollX - want) > 2 || w.scrollY != 0) {
                                w.post { if (holder.paged) w.scrollTo(holder.pageX(w, holder.page(w)), 0) }
                                return@setOnScrollChangeListener
                            }
                        }
                        model.report(holder.chapter, holder.fraction(w))
                    }
                    // A tap (not a scroll, not a link): paged, the side thirds turn
                    // the page; anywhere else it shows or hides the bars.
                    val taps = GestureDetector(ctx, object : GestureDetector.SimpleOnGestureListener() {
                        override fun onSingleTapConfirmed(e: MotionEvent): Boolean {
                            val hit = web.hitTestResult.type
                            if (hit == WebView.HitTestResult.SRC_ANCHOR_TYPE || hit == WebView.HitTestResult.SRC_IMAGE_ANCHOR_TYPE) return false
                            when (if (holder.paged) tapZone(e.x, web.width) else 0) {
                                -1 -> turn(false)
                                1 -> turn(true)
                                else -> chrome = !chrome
                            }
                            return false
                        }
                    })
                    val slop = ViewConfiguration.get(ctx).scaledTouchSlop
                    var downX = 0f
                    var downY = 0f
                    var dragged = false
                    setOnTouchListener { v, ev ->
                        taps.onTouchEvent(ev)
                        if (!holder.paged) return@setOnTouchListener false
                        when (ev.actionMasked) {
                            MotionEvent.ACTION_DOWN -> {
                                downX = ev.x
                                downY = ev.y
                                dragged = false
                                false
                            }
                            // The page never moves under a finger: a swipe turns it.
                            MotionEvent.ACTION_MOVE -> {
                                if (abs(ev.x - downX) > slop || abs(ev.y - downY) > slop) dragged = true
                                true
                            }
                            MotionEvent.ACTION_UP -> {
                                if (!dragged) return@setOnTouchListener false
                                // The page saw a press, not a tap: it must not follow a link.
                                val cancel = MotionEvent.obtain(ev)
                                cancel.action = MotionEvent.ACTION_CANCEL
                                v.onTouchEvent(cancel)
                                cancel.recycle()
                                when (swipeTurn(ev.x - downX, ev.y - downY, v.width)) {
                                    1 -> turn(true)
                                    -1 -> turn(false)
                                }
                                true
                            }
                            else -> false
                        }
                    }
                    holder.view = this
                }
            },
            update = { v ->
                holder.density = density
                val page = html
                val key = loadKey to css
                if (page != null && v.tag != key) {
                    // The same chapter in another style (layout, view size, theme): keep the place.
                    val old = v.tag as? Pair<*, *>
                    if (old != null && old.first == loadKey && holder.loaded && holder.pending == null) {
                        holder.pending = holder.fraction(v)
                        holder.pendingText = false
                    }
                    holder.paged = pagedCss != null
                    holder.loaded = false
                    holder.generation++
                    v.tag = key
                    v.settings.textZoom = zoom
                    v.loadDataWithBaseURL(base, page, "text/html", "utf-8", null)
                } else if (v.settings.textZoom != zoom) {
                    v.settings.textZoom = zoom
                    val gen = holder.generation
                    v.postDelayed({
                        if (gen == holder.generation && holder.loaded) {
                            holder.settle(v)
                            model.report(holder.chapter, holder.fraction(v))
                        }
                    }, 200)
                }
            },
            onRelease = { v ->
                holder.view = null
                v.destroy()
            },
            modifier = Modifier.fillMaxSize().onSizeChanged { viewport = it },
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
                        // The device test (app/src/androidTest) waits on this line.
                        Modifier.padding(top = 2.dp).testTag(BOOK_STATUS_TAG), color = l.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                }
                SizeBtn("A−", stringResource(R.string.book_text_smaller)) { setZoom(zoom - 10) }
                SizeBtn("A+", stringResource(R.string.book_text_larger)) { setZoom(zoom + 10) }
                IconBtn(
                    if (paged) Ic.Scroll else Ic.Pages,
                    stringResource(if (paged) R.string.book_layout_scroll else R.string.book_layout_paged),
                    { setLayout(!paged) },
                    tint = l.text,
                )
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

/** The book reader's "Page X of Y · Chapter N of M" line, for the device test. */
const val BOOK_STATUS_TAG = "book-status"

/** A small text button of the bar (the text size). */
@Composable
private fun SizeBtn(text: String, label: String, onClick: () -> Unit) {
    val l = LocalLain.current
    Box(
        Modifier.size(40.dp).clip(l.buttonShape).clickable(onClickLabel = label, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { Text(text, color = l.text, fontSize = 13.sp, fontWeight = FontWeight.SemiBold) }
}
