package io.github.enrell.lain.ui.screens

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
fun ReaderScreen(core: LainCoreInterface, itemId: String, title: String = "", onBack: () -> Unit = {}) {
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
    Box(
        Modifier
            .fillMaxSize()
            .background(l.bg)
            .focusRequester(focus)
            .focusable()
            .onPreviewKeyEvent { e ->
                if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                when (e.key) {
                    Key.DirectionRight -> if (state.rtl) model.previous() else model.next()
                    Key.DirectionLeft -> if (state.rtl) model.next() else model.previous()
                    Key.Spacebar, Key.PageDown, Key.DirectionDown -> model.next()
                    Key.PageUp, Key.DirectionUp -> model.previous()
                    Key.MoveHome -> model.goTo(0)
                    Key.MoveEnd -> model.goTo(book.pages.size - 1)
                    else -> return@onPreviewKeyEvent false
                }
                true
            },
    ) {
        HorizontalPager(state = pager, reverseLayout = state.rtl, beyondViewportPageCount = 1, modifier = Modifier.fillMaxSize()) { i ->
            val bmp: ImageBitmap? by produceState<ImageBitmap?>(null, i) {
                val bytes = model.page(i)
                value = bytes?.let { withContext(Dispatchers.Default) { decode(it, 2400) } }
            }
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                bmp?.let { Image(it, contentDescription = null, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize()) }
                    ?: CircularProgressIndicator(Modifier.size(36.dp), color = l.muted, strokeWidth = 2.dp)
            }
        }
        // A tap on a side turns the page (by reading direction); the middle shows or hides the bars.
        Row(Modifier.fillMaxSize()) {
            for (side in listOf(-1, 0, 1)) {
                Box(
                    Modifier.weight(if (side == 0) 0.4f else 0.3f).fillMaxHeight().pointerInput(side, state.rtl) {
                        detectTapGestures {
                            when {
                                side == 0 -> chrome = !chrome
                                (side > 0) != state.rtl -> model.next()
                                else -> model.previous()
                            }
                        }
                    },
                )
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
                    Mono(if (book.offline) stringResource(R.string.reader_offline) else stringResource(if (state.rtl) R.string.reader_rtl else R.string.reader_ltr), color = l.accent)
                    Text(title, color = l.text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Text(stringResource(R.string.reader_pages, state.page + 1, book.pages.size), Modifier.padding(horizontal = 12.dp), style = TextStyle(fontFamily = l.mono, fontSize = 12.sp, color = l.muted))
            }
        }
        AnimatedVisibility(chrome && book.pages.size > 1, Modifier.align(Alignment.BottomCenter), enter = fadeIn() + slideInVertically { it / 2 }, exit = fadeOut() + slideOutVertically { it / 2 }) {
            Row(
                Modifier.safeDrawingPadding().padding(12.dp).fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(l.bg.copy(alpha = 0.88f)).border(1.dp, l.line.copy(alpha = 0.7f), RoundedCornerShape(16.dp)).padding(horizontal = 16.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                val last = book.pages.size - 1
                Text(if (state.rtl) book.pages.size.toString() else "1", style = TextStyle(fontFamily = l.mono, fontSize = 11.sp, color = l.muted))
                ThinSlider(
                    state.page.toFloat() / last,
                    onChange = {},
                    onCommit = { f -> model.goTo((f * last).roundToInt().coerceIn(0, last)) },
                    modifier = Modifier.weight(1f).padding(horizontal = 12.dp),
                    reversed = state.rtl,
                    label = stringResource(R.string.progress_page, state.page + 1, book.pages.size),
                )
                Text(if (state.rtl) "1" else book.pages.size.toString(), style = TextStyle(fontFamily = l.mono, fontSize = 11.sp, color = l.muted))
            }
        }
    }
}
