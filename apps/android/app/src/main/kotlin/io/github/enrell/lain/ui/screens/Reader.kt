package io.github.enrell.lain.ui.screens

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
fun ReaderScreen(core: LainCoreInterface, itemId: String) {
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
                    ?: Text(stringResource(R.string.working), color = l.muted)
            }
        }
        Row(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().background(l.scrim).padding(l.sp(2)),
            horizontalArrangement = Arrangement.spacedBy(l.sp(2)),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Label(stringResource(R.string.progress_page, state.page + 1, book.pages.size))
            if (book.offline) Label(stringResource(R.string.reader_offline), color = l.ok)
            Label(book.direction)
            Kbd(if (state.rtl) "←" else "→")
            Kbd("Space")
            Kbd("Esc")
        }
    }
}
