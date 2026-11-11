package io.github.enrell.lain.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.enrell.lain.R
import io.github.enrell.lain.core.CopyState
import io.github.enrell.lain.core.Item
import io.github.enrell.lain.domain.Card
import io.github.enrell.lain.domain.RailLabel
import io.github.enrell.lain.domain.Title
import io.github.enrell.lain.domain.buildRails
import io.github.enrell.lain.domain.fraction
import io.github.enrell.lain.domain.nextUp
import io.github.enrell.lain.domain.reading
import io.github.enrell.lain.domain.titleKey
import io.github.enrell.lain.ui.AppState
import io.github.enrell.lain.ui.AppViewModel
import io.github.enrell.lain.ui.Screen
import io.github.enrell.lain.ui.theme.LocalLain

/** Home: a title-first hero, then Continue, Recently added and one rail per library. */
@Composable
fun HomeScreen(state: AppState, vm: AppViewModel) {
    val l = LocalLain.current
    val rails = remember(state.titles, state.continues, state.libraries) {
        buildRails(state.titles, state.continues, state.items, state.libraries)
    }
    if (rails.isEmpty()) {
        Centered {
            Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(stringResource(if (state.busy) R.string.working else R.string.home_empty), color = l.muted)
                ErrorLine(state.error)
            }
        }
        return
    }
    val hero: Title? = rails.firstNotNullOfOrNull { r ->
        r.cards.firstNotNullOfOrNull { c ->
            when (c) {
                is Card.Resume -> state.titles.firstOrNull { it.key == titleKey(c.item) }
                is Card.Show -> c.title
            }
        }
    }
    val heroH = (LocalConfiguration.current.screenHeightDp * l.tokens.heroHeight).dp
    LazyColumn(contentPadding = PaddingValues(bottom = l.sp(5))) {
        if (hero != null) {
            item {
                Box(Modifier.fillMaxWidth().height(heroH)) {
                    Art(hero.items.first().id, Modifier.fillMaxSize())
                    Box(Modifier.fillMaxSize().background(heroScrim(l.heroStops, l.bg)))
                    Column(Modifier.align(Alignment.BottomStart).padding(l.sp(4)), verticalArrangement = Arrangement.spacedBy(l.sp(2))) {
                        Label(if (hero.reading) stringResource(R.string.kind_reading) else stringResource(R.string.kind_watching))
                        Text(hero.title, style = MaterialTheme.typography.displayLarge, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        Row(horizontalArrangement = Arrangement.spacedBy(l.sp(2))) {
                            Action(
                                stringResource(if (hero.reading) R.string.action_read else R.string.action_play),
                                "Enter",
                                { vm.open(nextUp(hero, state.progress)) },
                                primary = true,
                            )
                            Action(stringResource(R.string.action_details), null, { vm.go(Screen.TitleDetail(hero.key)) })
                        }
                    }
                }
            }
        }
        if (state.fromCache) item { OfflineNote(state) }
        items(rails) { rail ->
            Column(Modifier.padding(top = l.sp(4))) {
                Label(
                    when (val lab = rail.label) {
                        RailLabel.Continue -> stringResource(R.string.rail_continue)
                        RailLabel.Recent -> stringResource(R.string.rail_recent)
                        is RailLabel.Library -> lab.name
                    },
                    Modifier.padding(start = l.sp(4), bottom = l.sp(2)),
                )
                LazyRow(contentPadding = PaddingValues(horizontal = l.sp(4)), horizontalArrangement = Arrangement.spacedBy(l.gap)) {
                    items(rail.cards) { card ->
                        when (card) {
                            is Card.Resume -> PosterCard(
                                artId = card.item.id,
                                title = card.item.title,
                                subtitle = episodeLabel(card.item.season, card.item.episode, card.item.reading()),
                                progress = fraction(card.progress),
                                onClick = { vm.open(card.item) },
                                width = l.cardW * 1.6f,
                                height = l.cardW * 0.9f,
                            )
                            is Card.Show -> PosterCard(
                                artId = card.title.items.first().id,
                                title = card.title.title,
                                subtitle = if (card.title.year > 0) card.title.year.toString() else "",
                                progress = 0f,
                                onClick = { vm.go(Screen.TitleDetail(card.title.key)) },
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun OfflineNote(state: AppState) {
    val l = LocalLain.current
    Row(Modifier.padding(horizontal = l.sp(4), vertical = l.sp(2)), horizontalArrangement = Arrangement.spacedBy(l.sp(2)), verticalAlignment = Alignment.CenterVertically) {
        Dot(l.warn)
        Text(stringResource(R.string.offline_snapshot), color = l.muted, style = MaterialTheme.typography.bodySmall)
    }
}

/** The Library grid (all titles) or Read (comics and manga only). */
@Composable
fun LibraryScreen(state: AppState, vm: AppViewModel, readingOnly: Boolean) {
    val l = LocalLain.current
    val titles = remember(state.titles, readingOnly) { if (readingOnly) state.titles.filter { it.reading } else state.titles }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.padding(l.sp(4)), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(l.sp(2))) {
            Text(stringResource(if (readingOnly) R.string.nav_read else R.string.nav_library), style = MaterialTheme.typography.headlineLarge)
            Label(pluralStringResource(R.plurals.count_titles, titles.size, titles.size))
        }
        if (state.fromCache) OfflineNote(state)
        if (titles.isEmpty()) {
            Centered { Text(stringResource(R.string.library_empty), color = l.muted) }
            return
        }
        LazyVerticalGrid(
            columns = GridCells.Adaptive(l.cardW),
            contentPadding = PaddingValues(l.sp(4)),
            horizontalArrangement = Arrangement.spacedBy(l.gap),
            verticalArrangement = Arrangement.spacedBy(l.gap),
        ) {
            items(titles, key = { it.key }) { t ->
                PosterCard(
                    artId = t.items.first().id,
                    title = t.title,
                    subtitle = pluralStringResource(R.plurals.count_items, t.items.size, t.items.size),
                    progress = 0f,
                    onClick = { vm.go(Screen.TitleDetail(t.key)) },
                )
            }
        }
    }
}

/** A title: hero art, the main action, the episode or volume list with progress and downloads. */
@Composable
fun TitleScreen(state: AppState, vm: AppViewModel, key: String, onDownload: (Item) -> Unit) {
    val l = LocalLain.current
    val title = state.titles.firstOrNull { it.key == key }
    if (title == null) {
        Centered { Text(stringResource(R.string.err_not_found), color = l.muted) }
        return
    }
    LaunchedEffect(key) {
        vm.loadProgress(title.items.map { it.id })
        vm.loadCopies()
    }
    LazyColumn(contentPadding = PaddingValues(bottom = l.sp(5))) {
        item {
            Box(Modifier.fillMaxWidth().height(260.dp)) {
                Art(title.items.first().id, Modifier.fillMaxSize())
                Box(Modifier.fillMaxSize().background(heroScrim(l.heroStops, l.bg)))
                Column(Modifier.align(Alignment.BottomStart).padding(l.sp(4)), verticalArrangement = Arrangement.spacedBy(l.sp(2))) {
                    Label(if (title.year > 0) "${title.kind} · ${title.year}" else title.kind)
                    Text(title.title, style = MaterialTheme.typography.displayMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    val next = nextUp(title, state.progress)
                    Action(
                        stringResource(if (title.reading) R.string.action_read else R.string.action_play) +
                            episodeLabel(next.season, next.episode, title.reading).let { if (it.isEmpty()) "" else " · $it" },
                        "Enter",
                        { vm.open(next) },
                        primary = true,
                    )
                }
            }
        }
        items(title.items, key = { it.id }) { item ->
            ItemRow(item, state, vm, onDownload)
        }
    }
}

@Composable
private fun ItemRow(item: Item, state: AppState, vm: AppViewModel, onDownload: (Item) -> Unit) {
    val l = LocalLain.current
    val p = state.progress[item.id]
    val copy = state.copies[item.id]
    Row(
        Modifier.fillMaxWidth().padding(horizontal = l.sp(4), vertical = l.sp(1)),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(l.sp(2)),
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                episodeLabel(item.season, item.episode, item.reading()).ifEmpty { item.title },
                style = MaterialTheme.typography.titleSmall,
            )
            Text(item.fileName, color = l.muted, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (p != null && (p.completed || p.positionSec > 0)) {
                Label(
                    when {
                        p.completed -> stringResource(R.string.progress_done)
                        item.reading() -> stringResource(R.string.progress_page, p.positionSec.toInt(), p.durationSec.toInt())
                        else -> stringResource(R.string.progress_at, clock(p.positionSec))
                    },
                    color = if (p.completed) l.ok else l.accent,
                )
            }
            when (copy) {
                null -> Unit
                CopyState.DONE -> Label(stringResource(R.string.reader_offline), color = l.ok)
                else -> Label(stringResource(R.string.copy_pending))
            }
        }
        Action(stringResource(if (item.reading()) R.string.action_read else R.string.action_play), null, { vm.open(item) })
        if (copy != CopyState.DONE) Action(stringResource(R.string.action_download), null, { onDownload(item) })
    }
}

/** Search: the server's search, or offline a title match over the snapshot. */
@Composable
fun SearchScreen(state: AppState, vm: AppViewModel) {
    val l = LocalLain.current
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    Column(Modifier.fillMaxSize().padding(l.sp(4)), verticalArrangement = Arrangement.spacedBy(l.sp(3))) {
        OutlinedTextField(
            value = state.query,
            onValueChange = { vm.search(it) },
            label = { Text(stringResource(R.string.search_hint)) },
            singleLine = true,
            modifier = Modifier.fillMaxWidth().focusRequester(focus),
        )
        if (state.searching) Label(stringResource(R.string.working))
        LazyColumn(verticalArrangement = Arrangement.spacedBy(l.sp(1))) {
            items(state.results, key = { it.id }) { item ->
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(l.sp(2))) {
                    Column(Modifier.weight(1f)) {
                        Text(item.title, style = MaterialTheme.typography.titleSmall)
                        Label(listOf(item.kind, episodeLabel(item.season, item.episode, item.reading())).filter { it.isNotEmpty() }.joinToString(" · "))
                    }
                    Action(stringResource(R.string.action_details), null, { vm.go(Screen.TitleDetail(titleKey(item))) })
                    Action(stringResource(if (item.reading()) R.string.action_read else R.string.action_play), null, { vm.open(item) })
                }
            }
        }
        if (!state.searching && state.query.isNotBlank() && state.results.isEmpty()) {
            Text(stringResource(R.string.search_none), color = l.muted)
        }
        Spacer(Modifier.height(l.sp(4)))
    }
}

/** `1:02:03` or `4:05`. */
fun clock(sec: Double): String {
    val s = sec.toLong().coerceAtLeast(0)
    val h = s / 3600
    val m = (s % 3600) / 60
    val r = s % 60
    return if (h > 0) "%d:%02d:%02d".format(h, m, r) else "%d:%02d".format(m, r)
}
