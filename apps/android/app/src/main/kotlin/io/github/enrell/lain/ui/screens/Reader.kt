package io.github.enrell.lain.ui.screens

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
    val pager = rememberPagerState(initialPage = state.page) { book.pages.size }
    LaunchedEffect(pager.currentPage) { model.goTo(pager.currentPage) }
    LaunchedEffect(state.page) {
        if (pager.currentPage != state.page) pager.animateScrollToPage(state.page)
    }
    LaunchedEffect(Unit) { focus.requestFocus() }
    var chrome by remember { mutableStateOf(true) }
    // Turning past the last page is the end of the volume, said so.
    var atEnd by remember(itemId) { mutableStateOf(false) }
    val prefs = androidx.compose.ui.platform.LocalContext.current.graph.prefs
    var hint by remember { mutableStateOf(!prefs.readerHintSeen) }
    LaunchedEffect(hint) {
        if (hint) {
            kotlinx.coroutines.delay(5000)
            hint = false
            prefs.readerHintSeen = true
        }
    }
    fun forward() {
        hint = false
        if (state.page >= book.pages.size - 1) atEnd = true else model.next()
    }
    fun backward() {
        hint = false
        atEnd = false
        model.previous()
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
            val bmp: ImageBitmap? by produceState<ImageBitmap?>(null, i) {
                val bytes = model.page(i)
                value = bytes?.let { withContext(Dispatchers.Default) { decode(it, 2400) } }
            }
            // A tap on a side turns the page (by reading direction) and the middle
            // shows or hides the bars; a swipe goes through to the pager.
            Box(
                Modifier.fillMaxSize().pointerInput(rtl) {
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
                bmp?.let { Image(it, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize()) }
                    ?: CircularProgressIndicator(Modifier.size(36.dp), color = l.muted, strokeWidth = 2.dp)
            }
        }
        if (hint && !atEnd) {
            Text(
                stringResource(R.string.reader_hint),
                Modifier.align(Alignment.Center).padding(horizontal = 36.dp).clip(RoundedCornerShape(12.dp)).background(Color.Black.copy(alpha = 0.72f)).padding(horizontal = 16.dp, vertical = 12.dp),
                color = Color.White, fontSize = 13.sp, lineHeight = 19.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )
        }
        if (atEnd) {
            Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.6f)).pointerInput(Unit) { detectTapGestures { atEnd = false } }, contentAlignment = Alignment.Center) {
                Column(
                    Modifier.padding(horizontal = 32.dp).clip(RoundedCornerShape(16.dp)).background(l.bg).border(1.dp, l.line, RoundedCornerShape(16.dp)).padding(22.dp),
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
                // The direction is a control: on when pages run right to left.
                Row(
                    Modifier.padding(end = 6.dp).height(40.dp).clip(l.buttonShape).background(if (rtl) l.raised else Color.Transparent).border(1.dp, if (rtl) l.accent.copy(alpha = 0.55f) else l.line, l.buttonShape)
                        .clickable(onClickLabel = stringResource(R.string.reader_rtl)) { rtl = !rtl }.padding(horizontal = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(if (rtl) Ic.ArrowLeft else Ic.ChevronRight, null, tint = if (rtl) l.accent else l.muted, modifier = Modifier.size(14.dp))
                    Text(stringResource(if (rtl) R.string.reader_rtl else R.string.reader_ltr), Modifier.padding(start = 6.dp), color = if (rtl) l.text else l.muted, fontSize = 12.sp, fontWeight = FontWeight.Medium, maxLines = 1)
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
