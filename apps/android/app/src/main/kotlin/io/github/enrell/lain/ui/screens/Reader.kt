package io.github.enrell.lain.ui.screens

import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.graphics.graphicsLayer
import kotlinx.coroutines.launch
import androidx.compose.material3.Icon
import androidx.compose.foundation.shape.CircleShape
import io.github.enrell.lain.graph
import androidx.compose.foundation.systemGestureExclusion
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.clickable
import androidx.compose.ui.graphics.Color
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.math.roundToInt
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.stringResource
import io.github.enrell.lain.R
import io.github.enrell.lain.core.LainCoreInterface
import io.github.enrell.lain.decode
import io.github.enrell.lain.ui.ReaderModel
import io.github.enrell.lain.ui.theme.LocalLain
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.withContext

/**
 * The comic and manga reader: one page fit to the screen, swiped or
 * turned with the keys in the book's direction (`rtl` for manga).
 * Keys: ←/→ (by direction), Space/PageDown next, PageUp back, Home/End.
 */
@Composable
fun ReaderScreen(
    core: LainCoreInterface,
    itemId: String,
    title: String = "",
    onBack: () -> Unit = {},
    /** The volume after this one in its title, if any. */
    next: io.github.enrell.lain.core.Item? = null,
    onNext: (io.github.enrell.lain.core.Item) -> Unit = {},
) {
    val l = LocalLain.current
    val scope = remember(itemId) { CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate) }
    val model = remember(itemId) { ReaderModel(core, itemId, scope) }
    val state by model.state.collectAsState()
    val focus = remember { FocusRequester() }
    LaunchedEffect(itemId) { model.load() }
    DisposableEffect(itemId) {
        onDispose {
            val job = model.close()
            if (job == null) scope.cancel() else job.invokeOnCompletion { scope.cancel() }
        }
    }
    val book = state.book
    if (book == null) {
        Centered {
            if (state.error != null) ErrorLine(state.error) else Text(stringResource(R.string.working), color = l.muted)
        }
        return
    }
    if (book.pages.isEmpty()) {
        Centered { Text(stringResource(R.string.reader_empty), color = l.muted) }
        return
    }
    // One page past the last is the end of the volume: a swipe reaches it as a tap does.
    val pager = rememberPagerState(initialPage = state.page) { book.pages.size + 1 }
    // The page that came to rest is the page being read (the ones a jump passes over are not).
    LaunchedEffect(pager.settledPage) { if (pager.settledPage < book.pages.size) model.goTo(pager.settledPage) }
    LaunchedEffect(state.page) {
        if (pager.settledPage != state.page) pager.scrollToPage(state.page)
    }
    val turns = androidx.compose.runtime.rememberCoroutineScope()
    val atEnd = pager.currentPage >= book.pages.size
    LaunchedEffect(Unit) { focus.requestFocus() }
    var chrome by remember { mutableStateOf(true) }
    val prefs = androidx.compose.ui.platform.LocalContext.current.graph.prefs
    var hint by remember { mutableStateOf(!prefs.readerHintSeen) }
    // A book that runs right to left says so as it opens: next is on the left.
    var directionNote by remember(itemId) { mutableStateOf(book.direction == "rtl") }
    LaunchedEffect(directionNote) {
        if (directionNote) {
            kotlinx.coroutines.delay(3500)
            directionNote = false
        }
    }
    LaunchedEffect(hint) {
        if (hint) {
            kotlinx.coroutines.delay(5000)
            hint = false
            prefs.readerHintSeen = true
        }
    }
    fun forward() {
        hint = false
        if (atEnd) return
        turns.launch { pager.animateScrollToPage(pager.currentPage + 1) }
    }
    fun backward() {
        hint = false
        if (pager.currentPage > 0) turns.launch { pager.animateScrollToPage(pager.currentPage - 1) }
    }
    // The book's own direction, which the reader may turn around.
    var rtl by remember(book.direction) { mutableStateOf(book.direction == "rtl") }
    val activity = androidx.compose.ui.platform.LocalContext.current as? android.app.Activity
    DisposableEffect(Unit) {
        val bars = activity?.window?.let { androidx.core.view.WindowCompat.getInsetsController(it, it.decorView) }
        bars?.systemBarsBehavior = androidx.core.view.WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        bars?.hide(androidx.core.view.WindowInsetsCompat.Type.systemBars())
        onDispose { bars?.show(androidx.core.view.WindowInsetsCompat.Type.systemBars()) }
    }
    Box(
        Modifier
            .fillMaxSize()
            .background(l.bg)
            .focusRequester(focus)
            .focusable()
            .onPreviewKeyEvent { e ->
                if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                when (e.key) {
                    Key.DirectionRight -> if (rtl) backward() else forward()
                    Key.DirectionLeft -> if (rtl) forward() else backward()
                    Key.Spacebar, Key.PageDown, Key.DirectionDown -> forward()
                    Key.PageUp, Key.DirectionUp -> backward()
                    Key.MoveHome -> model.goTo(0)
                    Key.MoveEnd -> model.goTo(book.pages.size - 1)
                    else -> return@onPreviewKeyEvent false
                }
                true
            },
    ) {
        HorizontalPager(state = pager, reverseLayout = rtl, beyondViewportPageCount = 1, modifier = Modifier.fillMaxSize()) { i ->
            if (i >= book.pages.size) {
                // The end of the volume: said, with where to go from here.
                Box(Modifier.fillMaxSize().pointerInput(rtl) { detectTapGestures { o -> if ((o.x / size.width > 0.7f) == rtl || (o.x / size.width < 0.3f) != rtl) backward() } }, contentAlignment = Alignment.Center) {
                    Column(
                        Modifier.padding(horizontal = 32.dp).clip(RoundedCornerShape(16.dp)).background(l.surface).border(1.dp, l.line, RoundedCornerShape(16.dp)).padding(22.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Text(stringResource(R.string.end_of_volume), color = l.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                        Text(title, Modifier.padding(top = 4.dp), color = l.muted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Row(Modifier.padding(top = 18.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                            WButton(stringResource(R.string.player_back), onBack, kind = Btn.Secondary)
                            if (next != null) WButton(stringResource(R.string.next_volume), { onNext(next) }, icon = Ic.ChevronRight)
                        }
                    }
                }
                return@HorizontalPager
            }
            val bmp: ImageBitmap? by produceState<ImageBitmap?>(null, i) {
                val bytes = model.page(i)
                value = bytes?.let { withContext(Dispatchers.Default) { decode(it, 2400) } }
            }
            // A tap on a side turns the page (by reading direction) and the middle
            // shows or hides the bars; a swipe goes through to the pager.
            // Two fingers zoom into the page (up to 4x) and one then moves around
            // it; at its own size a drag is the pager's, so swipes still turn pages.
            var zoom by remember(i) { mutableStateOf(1f) }
            var pan by remember(i) { mutableStateOf(androidx.compose.ui.geometry.Offset.Zero) }
            LaunchedEffect(pager.settledPage) { if (pager.settledPage != i) { zoom = 1f; pan = androidx.compose.ui.geometry.Offset.Zero } }
            Box(
                Modifier.fillMaxSize()
                    .pointerInput(i) {
                        awaitEachGesture {
                            awaitFirstDown(requireUnconsumed = false)
                            do {
                                val event = awaitPointerEvent()
                                if (event.changes.count { it.pressed } > 1 || zoom > 1f) {
                                    zoom = (zoom * event.calculateZoom()).coerceIn(1f, 4f)
                                    val maxX = size.width * (zoom - 1f) / 2f
                                    val maxY = size.height * (zoom - 1f) / 2f
                                    val moved = pan + event.calculatePan()
                                    pan = if (zoom == 1f) androidx.compose.ui.geometry.Offset.Zero else androidx.compose.ui.geometry.Offset(moved.x.coerceIn(-maxX, maxX), moved.y.coerceIn(-maxY, maxY))
                                    event.changes.forEach { if (it.positionChanged()) it.consume() }
                                }
                            } while (event.changes.any { it.pressed })
                        }
                    }
                    .pointerInput(rtl) {
                        detectTapGestures { o ->
                            val x = o.x / size.width
                            when {
                                x in 0.3f..0.7f -> chrome = !chrome
                                (x > 0.7f) != rtl -> forward()
                                else -> backward()
                            }
                        }
                    },
                contentAlignment = Alignment.Center,
            ) {
                bmp?.let {
                    Image(
                        it, contentDescription = null, contentScale = ContentScale.Fit,
                        modifier = Modifier.fillMaxSize().graphicsLayer { scaleX = zoom; scaleY = zoom; translationX = pan.x; translationY = pan.y },
                    )
                }
                    ?: CircularProgressIndicator(Modifier.size(36.dp), color = l.muted, strokeWidth = 2.dp)
            }
        }
        if (directionNote && !hint && !atEnd) {
            Row(
                Modifier.align(Alignment.Center).clip(CircleShape).background(Color.Black.copy(alpha = 0.75f)).padding(horizontal = 16.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Icon(Ic.ArrowLeft, null, tint = Color.White, modifier = Modifier.size(16.dp))
                Text(stringResource(R.string.reader_rtl_note), color = Color.White, fontSize = 13.sp, fontWeight = FontWeight.Medium)
            }
        }
        if (hint && !atEnd) {
            Text(
                stringResource(R.string.reader_hint),
                Modifier.align(Alignment.Center).padding(horizontal = 36.dp).clip(RoundedCornerShape(12.dp)).background(Color.Black.copy(alpha = 0.72f)).padding(horizontal = 16.dp, vertical = 12.dp),
                color = Color.White, fontSize = 13.sp, lineHeight = 19.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )
        }
        // The bars: the web reader's glass panels.
        AnimatedVisibility(chrome, Modifier.align(Alignment.TopCenter), enter = fadeIn() + slideInVertically { -it / 2 }, exit = fadeOut() + slideOutVertically { -it / 2 }) {
            Row(
                Modifier.safeDrawingPadding().padding(12.dp).fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(l.bg.copy(alpha = 0.88f)).border(1.dp, l.line.copy(alpha = 0.7f), RoundedCornerShape(16.dp)).padding(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconBtn(Ic.ArrowLeft, "Back", onBack, tint = l.text)
                Column(Modifier.weight(1f).padding(horizontal = 6.dp)) {
                    Text(title, color = l.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(
                        stringResource(R.string.progress_page, state.page + 1, book.pages.size) + if (book.offline) " · " + stringResource(R.string.reader_offline) else "",
                        Modifier.padding(top = 2.dp), color = l.muted, fontSize = 12.sp, maxLines = 1,
                    )
                }
                // The direction is a choice between two, the one in force lit.
                Row(Modifier.padding(end = 6.dp).height(40.dp).clip(l.buttonShape).border(1.dp, l.line, l.buttonShape)) {
                    for ((toRtl, label) in listOf(false to "L \u2192 R", true to "R \u2190 L")) {
                        val on = rtl == toRtl
                        Box(
                            Modifier.fillMaxHeight().background(if (on) l.accent.copy(alpha = 0.22f) else Color.Transparent)
                                .clickable(onClickLabel = stringResource(if (toRtl) R.string.reader_rtl else R.string.reader_ltr)) { rtl = toRtl }
                                .semantics { contentDescription = if (toRtl) "Right to left" else "Left to right"; stateDescription = if (on) "selected" else "not selected" }
                                .padding(horizontal = 11.dp),
                            contentAlignment = Alignment.Center,
                        ) { Text(label, color = if (on) l.text else l.muted, fontSize = 12.sp, fontWeight = if (on) FontWeight.SemiBold else FontWeight.Medium, maxLines = 1) }
                    }
                }
            }
        }
        AnimatedVisibility(chrome && book.pages.size > 1, Modifier.align(Alignment.BottomCenter), enter = fadeIn() + slideInVertically { it / 2 }, exit = fadeOut() + slideOutVertically { it / 2 }) {
            Row(
                // Clear of the system's bottom gesture strip: with the bars hidden the
                // safe area is the whole screen, and a drag there opened Recents.
                Modifier.safeDrawingPadding().padding(start = 12.dp, end = 12.dp, top = 12.dp, bottom = 40.dp).fillMaxWidth().systemGestureExclusion().clip(RoundedCornerShape(16.dp)).background(l.bg.copy(alpha = 0.88f)).border(1.dp, l.line.copy(alpha = 0.7f), RoundedCornerShape(16.dp)).padding(horizontal = 16.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                val last = book.pages.size - 1
                Text(
                    if (rtl) book.pages.size.toString() else "1",
                    Modifier.clip(CircleShape).clickable { model.goTo(if (rtl) last else 0) }.padding(horizontal = 8.dp, vertical = 10.dp),
                    style = TextStyle(fontFamily = l.mono, fontSize = 12.sp, color = l.muted),
                )
                ThinSlider(
                    state.page.toFloat() / last,
                    onChange = {},
                    onCommit = { f -> model.goTo((f * last).roundToInt().coerceIn(0, last)) },
                    modifier = Modifier.weight(1f).padding(horizontal = 6.dp),
                    reversed = rtl,
                    label = stringResource(R.string.progress_page, state.page + 1, book.pages.size),
                )
                Text(
                    if (rtl) "1" else book.pages.size.toString(),
                    Modifier.clip(CircleShape).clickable { model.goTo(if (rtl) 0 else last) }.padding(horizontal = 8.dp, vertical = 10.dp),
                    style = TextStyle(fontFamily = l.mono, fontSize = 12.sp, color = l.muted),
                )
            }
        }
    }
}
