package io.github.enrell.lain.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.enrell.lain.R
import io.github.enrell.lain.core.Item
import io.github.enrell.lain.core.ProgressInfo
import io.github.enrell.lain.domain.ADDED_DAYS
import io.github.enrell.lain.domain.Axis
import io.github.enrell.lain.domain.Card
import io.github.enrell.lain.domain.Filters
import io.github.enrell.lain.domain.Form
import io.github.enrell.lain.domain.RailLabel
import io.github.enrell.lain.domain.Sort
import io.github.enrell.lain.domain.Title
import io.github.enrell.lain.domain.Watch
import io.github.enrell.lain.domain.applyFilters
import io.github.enrell.lain.domain.buildRails
import io.github.enrell.lain.domain.countBy
import io.github.enrell.lain.domain.fraction
import io.github.enrell.lain.domain.genreFacets
import io.github.enrell.lain.domain.nextUp
import io.github.enrell.lain.domain.reading
import io.github.enrell.lain.domain.titleKey
import io.github.enrell.lain.domain.watchState
import io.github.enrell.lain.graph
import io.github.enrell.lain.ui.AppState
import io.github.enrell.lain.ui.AppViewModel
import io.github.enrell.lain.ui.Screen
import io.github.enrell.lain.ui.theme.LocalLain
import kotlinx.coroutines.launch

private val GUTTER = 20.dp
private val RAIL_H = 152.dp

private fun Title.art(): String = (items.firstOrNull { it.episode > 0 } ?: items.first()).id
private fun Title.wide(): Boolean = Shapes.landscape(art(), guess = !reading)

/** A title's genres across its files' overlays, first spelling kept. */
private fun AppState.genresOf(t: Title): List<String> {
    val seen = LinkedHashMap<String, String>()
    for (item in t.items) for (g in overlays[item.id]?.genres.orEmpty()) seen.getOrPut(g.trim().lowercase()) { g.trim() }
    return seen.values.filter { it.isNotEmpty() }
}

private fun AppState.synopsisOf(t: Title): String = t.items.firstNotNullOfOrNull { overlays[it.id]?.synopsis?.takeIf { s -> s.isNotBlank() } } ?: ""

/** Every progress record the app holds, by item. */
private fun AppState.allProgress(): Map<String, ProgressInfo> = continues.associateBy { it.itemId } + progress

@Composable
private fun titleSubtitle(t: Title): String = when {
    t.items.size > 1 && t.reading -> pluralStringResource(R.plurals.count_items, t.items.size, t.items.size)
    t.items.size > 1 -> pluralStringResource(R.plurals.count_episodes, t.items.size, t.items.size)
    t.year > 0 -> t.year.toString()
    else -> t.kind
}

/** A title's card at rail height: as wide as its picture's shape. */
@Composable
private fun RailCard(t: Title, vm: AppViewModel, height: Dp = RAIL_H) {
    val wide = t.wide()
    ShapeCard(
        artId = t.art(),
        title = t.title,
        subtitle = titleSubtitle(t),
        onClick = { vm.go(Screen.TitleDetail(t.key)) },
        modifier = Modifier.width(height * (if (wide) Shapes.LANDSCAPE else Shapes.PORTRAIT)),
        landscape = wide,
        badge = if (t.items.size > 1) t.items.size.toString() else null,
    )
}

@Composable
private fun RailHeader(title: String, action: String?, onClick: (() -> Unit)?) {
    val l = LocalLain.current
    Row(
        Modifier.fillMaxWidth().padding(horizontal = GUTTER).then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier).padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        SectionTitle(title)
        if (onClick != null) Icon(Ic.ChevronRight, null, tint = l.muted, modifier = Modifier.padding(start = 4.dp).size(16.dp))
        Box(Modifier.weight(1f))
        if (action != null) Mono(action, color = l.muted, size = 10)
    }
}

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
    val hero: Title? = remember(rails, state.titles) {
        rails.firstNotNullOfOrNull { r ->
            r.cards.firstNotNullOfOrNull { c ->
                when (c) {
                    is Card.Resume -> state.titles.firstOrNull { it.key == titleKey(c.item) }
                    is Card.Show -> c.title
                }
            }
        }
    }
    val heroH = (LocalConfiguration.current.screenHeightDp * 0.66f).dp
    LazyColumn(contentPadding = PaddingValues(bottom = 32.dp)) {
        if (hero != null) {
            item {
                Box(Modifier.fillMaxWidth().height(heroH)) {
                    Art(hero.art(), Modifier.fillMaxSize())
                    Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0f to Color.Transparent, 0.45f to l.bg.copy(alpha = 0.35f), 1f to l.bg)))
                    Column(Modifier.align(Alignment.BottomStart).padding(horizontal = GUTTER).padding(bottom = 20.dp)) {
                        Mono(
                            stringResource(R.string.hero_label) + if (hero.year > 0) " / ${hero.year}" else "",
                            color = l.accent,
                        )
                        Text(
                            hero.title.uppercase(),
                            Modifier.padding(top = 14.dp),
                            color = l.text, fontSize = 40.sp, lineHeight = 40.sp, fontWeight = FontWeight.Black, letterSpacing = (-1.6).sp,
                            maxLines = 3, overflow = TextOverflow.Ellipsis,
                        )
                        Mono(heroMeta(hero) + state.genresOf(hero).take(2).joinToString("") { " · $it" }, Modifier.padding(top = 14.dp), color = l.text.copy(alpha = 0.75f))
                        state.synopsisOf(hero).takeIf { it.isNotEmpty() }?.let { text ->
                            Text(text, Modifier.padding(top = 14.dp), color = l.text.copy(alpha = 0.7f), fontSize = 14.sp, lineHeight = 22.sp, maxLines = 3, overflow = TextOverflow.Ellipsis)
                        }
                        Row(Modifier.padding(top = 20.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Row(
                                Modifier.height(44.dp).clip(CircleShape).background(Color.White).clickable { vm.open(nextUp(hero, state.progress)) }.padding(horizontal = 24.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Icon(Ic.Play, null, tint = Color.Black, modifier = Modifier.size(14.dp))
                                Text(stringResource(if (hero.reading) R.string.action_read else R.string.action_play), color = Color.Black, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
                            }
                            Row(
                                Modifier.height(44.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.35f)).border(1.dp, Color.White.copy(alpha = 0.2f), CircleShape)
                                    .clickable { vm.go(Screen.TitleDetail(hero.key)) }.padding(horizontal = 20.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Icon(Ic.Info, null, tint = l.text, modifier = Modifier.size(16.dp))
                                Text(stringResource(R.string.action_details), color = l.text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
                            }
                        }
                    }
                }
            }
        }
        if (state.fromCache) item { OfflineNote(state) }
        items(rails) { rail ->
            Column(Modifier.padding(top = 28.dp)) {
                val lib = (rail.label as? RailLabel.Library)
                val libId = lib?.let { n -> state.libraries.firstOrNull { it.name == n.name }?.id }
                RailHeader(
                    when (val lab = rail.label) {
                        RailLabel.Continue -> stringResource(R.string.rail_continue)
                        RailLabel.Recent -> stringResource(R.string.rail_recent)
                        is RailLabel.Library -> lab.name
                    },
                    if (libId != null) stringResource(R.string.view_all) else null,
                    libId?.let { id -> { vm.go(Screen.LibraryDetail(id)) } },
                )
                LazyRow(Modifier.padding(top = 8.dp), contentPadding = PaddingValues(horizontal = GUTTER), horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                    items(rail.cards.take(20)) { card ->
                        when (card) {
                            is Card.Resume -> ShapeCard(
                                artId = card.item.id,
                                title = card.item.title,
                                subtitle = episodeText(card.item),
                                onClick = { vm.open(card.item) },
                                modifier = Modifier.width(RAIL_H * Shapes.LANDSCAPE),
                                landscape = true,
                                progress = fraction(card.progress),
                            )
                            is Card.Show -> RailCard(card.title, vm)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun heroMeta(t: Title): String = listOfNotNull(
    if (t.items.size > 1) titleSubtitle(t) else null,
    t.year.takeIf { it > 0 }?.toString(),
    t.kind,
).joinToString(" · ")

@Composable
private fun episodeText(item: Item): String =
    if (item.episode > 0) stringResource(if (item.reading()) R.string.volume_n else R.string.episode_n, item.episode.toInt()) else if (item.year > 0) item.year.toString() else item.kind

@Composable
fun OfflineNote(state: AppState) {
    val l = LocalLain.current
    Row(Modifier.padding(horizontal = GUTTER, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        Dot(l.warn)
        Text(stringResource(R.string.offline_snapshot), color = l.muted, fontSize = 12.sp)
    }
}

/** The Library front: one shelf per library, or every title in one grid. */
@Composable
fun LibraryScreen(state: AppState, vm: AppViewModel, readingOnly: Boolean) {
    val l = LocalLain.current
    val prefs = LocalContext.current.graph.prefs
    val titles = remember(state.titles, readingOnly) { if (readingOnly) state.titles.filter { it.reading } else state.titles }
    val libraries = remember(titles, state.libraries) { state.libraries.filter { lib -> titles.any { it.libraryId == lib.id } } }
    var shelves by rememberSaveable { mutableStateOf(prefs.libraryShelves) }
    var unlocking by remember { mutableStateOf(false) }
    val byShelf = libraries.size > 1 && shelves
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.padding(start = GUTTER, end = GUTTER, top = 18.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(if (readingOnly) R.string.nav_read else R.string.nav_library), Modifier.weight(1f), color = l.text, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.4).sp)
            // One quiet control for private libraries: it never names them
            // or says how many there are (D-156).
            val locked = state.libraries.any { it.private && it.locked }
            if (state.libraries.any { it.private }) {
                IconBtn(
                    if (locked) Ic.Lock else Ic.LockOpen,
                    stringResource(if (locked) R.string.unlock_title else R.string.lock_again),
                    { if (locked) unlocking = true else vm.lockAll() },
                    outlined = true,
                )
            }
        }
        if (unlocking) UnlockSheet(vm) { unlocking = false }
        if (libraries.size > 1) {
            Row(Modifier.padding(horizontal = GUTTER).padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                for ((on, label) in listOf(true to R.string.view_by_library, false to R.string.view_all_titles)) {
                    Box(
                        Modifier.clip(l.buttonShape).background(if (shelves == on) l.raised else Color.Transparent)
                            .clickable { shelves = on; prefs.libraryShelves = on }.padding(horizontal = 12.dp, vertical = 10.dp),
                    ) { Mono(stringResource(label), color = if (shelves == on) l.text else l.muted, size = 11) }
                }
            }
        }
        if (state.fromCache) OfflineNote(state)
        if (titles.isEmpty()) {
            Centered { Text(stringResource(R.string.library_empty), color = l.muted) }
            return
        }
        if (byShelf) {
            LazyColumn(contentPadding = PaddingValues(top = 12.dp, bottom = 32.dp)) {
                items(libraries, key = { it.id }) { lib ->
                    val mine = remember(titles, lib.id) { titles.filter { it.libraryId == lib.id }.sortedByDescending { it.latest } }
                    Column(Modifier.padding(top = 20.dp)) {
                        RailHeader(lib.name, pluralStringResource(R.plurals.count_items, mine.sumOf { it.items.size }, mine.sumOf { it.items.size })) {
                            vm.go(Screen.LibraryDetail(lib.id))
                        }
                        LazyRow(Modifier.padding(top = 8.dp), contentPadding = PaddingValues(horizontal = GUTTER), horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                            items(mine.take(14), key = { it.key }) { RailCard(it, vm) }
                            if (mine.size > 14) {
                                item {
                                    Box(
                                        Modifier.height(RAIL_H).width(112.dp).clip(l.cardShape).border(1.dp, l.line, l.cardShape).clickable { vm.go(Screen.LibraryDetail(lib.id)) },
                                        contentAlignment = Alignment.Center,
                                    ) { Text(stringResource(R.string.see_all), color = l.muted, fontSize = 14.sp) }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            Box(Modifier.padding(top = 10.dp)) { LibraryBrowser(titles, state, vm, searchHint = stringResource(R.string.search_library)) }
        }
    }
}

/** One library: its name with the way back, then the browser. */
@Composable
fun LibraryDetailScreen(state: AppState, vm: AppViewModel, id: String) {
    val l = LocalLain.current
    val lib = state.libraries.firstOrNull { it.id == id }
    val titles = remember(state.titles, id) { state.titles.filter { it.libraryId == id } }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(start = 6.dp, end = GUTTER, top = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            IconBtn(Ic.ChevronLeft, stringResource(R.string.nav_library), { vm.back() })
            Column {
                Mono(stringResource(R.string.nav_library), size = 10)
                Text(lib?.name ?: "", color = l.text, fontSize = 20.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.4).sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        Box(Modifier.padding(top = 8.dp)) { LibraryBrowser(titles, state, vm, searchHint = stringResource(R.string.search_library)) }
    }
}

/**
 * A set of titles to search and narrow: a search field and a filters
 * sheet (sort, tile size, progress, form, kind, added), then a grid by
 * shape: a landscape still takes the row and portraits two a row, or
 * two and three with small tiles.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun LibraryBrowser(titles: List<Title>, state: AppState, vm: AppViewModel, searchHint: String) {
    val l = LocalLain.current
    val prefs = LocalContext.current.graph.prefs
    var filters by remember { mutableStateOf(Filters(sort = Sort.of(prefs.librarySort))) }
    var dense by remember { mutableStateOf(prefs.smallTiles) }
    var sheet by remember { mutableStateOf(false) }
    val now = remember { System.currentTimeMillis() / 1000 }
    val progress = remember(state.continues, state.progress) { state.allProgress() }
    val genresOf: (Title) -> List<String> = remember(state.overlays) { { t: Title -> state.genresOf(t) } }
    val results = remember(titles, filters, progress, state.overlays) { applyFilters(titles, filters, progress, now, genresOf) }
    val grid = rememberLazyGridState()
    val scope = rememberCoroutineScope()
    val far by remember { derivedStateOf { grid.firstVisibleItemIndex > 10 } }
    // A new result set starts at its beginning.
    LaunchedEffect(filters) { grid.scrollToItem(0) }

    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Row(Modifier.padding(horizontal = GUTTER), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                SearchField(filters.q, { filters = filters.copy(q = it) }, searchHint, Modifier.weight(1f))
                Box {
                    IconBtn(Ic.Sliders, stringResource(R.string.filters), { sheet = true }, tint = if (filters.active > 0) l.text else l.muted, outlined = true)
                    if (filters.active > 0) {
                        Box(Modifier.align(Alignment.TopEnd).padding(4.dp).size(8.dp).clip(CircleShape).background(l.accent))
                    }
                }
            }
            FlowRow(
                Modifier.padding(horizontal = GUTTER).padding(top = 10.dp, bottom = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Text(
                    if (filters.narrowed) stringResource(R.string.count_of, results.size, titles.size) else pluralStringResource(R.plurals.count_titles, titles.size, titles.size),
                    Modifier.padding(end = 4.dp).align(Alignment.CenterVertically),
                    color = l.muted, fontSize = 14.sp,
                )
                for (g in filters.genres) FilterChip(g) { filters = filters.copy(genres = filters.genres - g) }
                filters.watch?.let { w -> FilterChip(watchLabel(w)) { filters = filters.copy(watch = null) } }
                filters.form?.let { s -> FilterChip(formLabel(s)) { filters = filters.copy(form = null) } }
                if (filters.kind.isNotEmpty()) FilterChip(filters.kind) { filters = filters.copy(kind = "") }
                if (filters.added > 0) FilterChip(addedLabel(filters.added)) { filters = filters.copy(added = 0) }
                if (filters.narrowed) {
                    Text(stringResource(R.string.clear), Modifier.clip(CircleShape).clickable { filters = filters.cleared() }.padding(horizontal = 8.dp, vertical = 4.dp), color = l.muted, fontSize = 12.sp)
                }
            }
            if (results.isEmpty()) {
                Column(Modifier.fillMaxWidth().padding(top = 56.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        if (filters.q.isNotBlank()) stringResource(R.string.nothing_matches, filters.q.trim()) else stringResource(R.string.nothing_passes),
                        color = l.text, fontSize = 14.sp,
                    )
                    Gap(16.dp)
                    WButton(stringResource(R.string.clear_all), { filters = filters.cleared() }, kind = Btn.Secondary)
                }
            } else {
                // The shapes are read here, in composition, and handed to the grid
                // as a plain list: a lazy grid caches its rows from the spans, so
                // a span that changed behind its back (a cover that loaded and
                // turned a card from landscape to portrait) left an item in two
                // rows and crashed the layout. A new list rebuilds the rows.
                val wide = results.map { it.wide() }
                LazyVerticalGrid(
                    columns = GridCells.Fixed(6),
                    state = grid,
                    contentPadding = PaddingValues(start = GUTTER, end = GUTTER, top = 8.dp, bottom = 96.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalArrangement = Arrangement.spacedBy(20.dp),
                ) {
                    items(
                        results.size,
                        key = { results[it].key },
                        span = { GridItemSpan(if (wide[it]) (if (dense) 3 else 6) else (if (dense) 2 else 3)) },
                    ) { i ->
                        val t = results[i]
                        val p = t.items.firstNotNullOfOrNull { progress[it.id]?.takeIf { pr -> !pr.completed && pr.positionSec > 0 } }
                        ShapeCard(
                            artId = t.art(),
                            title = t.title,
                            subtitle = titleSubtitle(t),
                            onClick = { vm.go(Screen.TitleDetail(t.key)) },
                            landscape = wide[i],
                            progress = fraction(p),
                            badge = if (t.items.size > 1) t.items.size.toString() else null,
                        )
                    }
                }
            }
        }
        if (far) {
            Row(
                Modifier.align(Alignment.BottomEnd).padding(20.dp).height(40.dp).clip(CircleShape).background(l.surface.copy(alpha = 0.95f)).border(1.dp, l.line, CircleShape)
                    .clickable { scope.launch { grid.animateScrollToItem(0) } }.padding(horizontal = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Icon(Ic.ArrowUp, null, tint = l.muted, modifier = Modifier.size(16.dp))
                Text(stringResource(R.string.top), color = l.muted, fontSize = 14.sp)
            }
        }
    }

    if (sheet) {
        val watchCounts = countBy(titles, filters, progress, now, Axis.Watch, genresOf) { listOf(watchState(it, progress)) }
        val formCounts = countBy(titles, filters, progress, now, Axis.Form, genresOf) { listOf(if (it.items.size > 1) Form.Series else Form.Single) }
        val kindCounts = countBy(titles, filters, progress, now, Axis.Kind, genresOf) { t -> t.items.map { it.kind }.distinct() }
        val genres = remember(titles, filters, progress, state.overlays) { genreFacets(titles, filters, progress, now, genresOf) }
        var allGenres by remember { mutableStateOf(false) }
        val kinds = remember(titles) { titles.flatMap { t -> t.items.map { it.kind } }.distinct().filter { it.isNotEmpty() }.sorted() }
        Sheet(
            stringResource(R.string.filters),
            { sheet = false },
            footer = {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    WButton(stringResource(R.string.clear), { filters = filters.cleared() }, Modifier.weight(1f), kind = Btn.Ghost, enabled = filters.active > 0)
                    WButton(pluralStringResource(R.plurals.show_titles, results.size, results.size), { sheet = false }, Modifier.weight(1f))
                }
            },
        ) {
            SheetHeading(stringResource(R.string.sort))
            for (pair in Sort.entries.chunked(2)) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 3.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    for (s in pair) {
                        ChoiceTile(sortLabel(s), filters.sort == s, { filters = filters.copy(sort = s); prefs.librarySort = s.id }, Modifier.weight(1f))
                    }
                }
            }
            SheetHeading(stringResource(R.string.tiles))
            Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                ChoiceTile(stringResource(R.string.tiles_large), !dense, { dense = false; prefs.smallTiles = false }, Modifier.weight(1f))
                ChoiceTile(stringResource(R.string.tiles_small), dense, { dense = true; prefs.smallTiles = true }, Modifier.weight(1f))
            }
            SheetHeading(stringResource(R.string.progress))
            for (w in Watch.entries) {
                OptionRow(watchLabel(w), watchCounts[w] ?: 0, filters.watch == w, { filters = filters.copy(watch = if (filters.watch == w) null else w) }, enabled = (watchCounts[w] ?: 0) > 0)
            }
            if ((formCounts[Form.Series] ?: 0) > 0 && (formCounts[Form.Single] ?: 0) > 0) {
                SheetHeading(stringResource(R.string.form))
                for (s in Form.entries) {
                    OptionRow(formLabel(s), formCounts[s] ?: 0, filters.form == s, { filters = filters.copy(form = if (filters.form == s) null else s) })
                }
            }
            if (kinds.size > 1) {
                SheetHeading(stringResource(R.string.kind))
                for (k in kinds) {
                    OptionRow(k, kindCounts[k] ?: 0, filters.kind == k, { filters = filters.copy(kind = if (filters.kind == k) "" else k) }, enabled = (kindCounts[k] ?: 0) > 0)
                }
            }
            SheetHeading(stringResource(R.string.added))
            for (d in ADDED_DAYS) {
                val n = remember(titles, filters, d, state.overlays) { applyFilters(titles, filters.copy(added = d), progress, now, genresOf).size }
                OptionRow(addedLabel(d), n, filters.added == d, { filters = filters.copy(added = if (filters.added == d) 0 else d) }, enabled = n > 0)
            }
            if (genres.isNotEmpty()) {
                SheetHeading(stringResource(R.string.genres) + " · " + genres.size)
                for ((g, n) in if (allGenres) genres else genres.take(24)) {
                    val on = filters.genres.any { it.equals(g, ignoreCase = true) }
                    OptionRow(g, n, on, { filters = filters.copy(genres = if (on) filters.genres.filterNot { it.equals(g, ignoreCase = true) } else filters.genres + g) })
                }
                if (genres.size > 24) {
                    Text(
                        stringResource(if (allGenres) R.string.show_fewer else R.string.show_all_n, genres.size),
                        Modifier.clickable { allGenres = !allGenres }.padding(horizontal = 8.dp, vertical = 12.dp),
                        color = l.muted, fontSize = 12.sp,
                    )
                }
            }
        }
    }
}

@Composable
private fun sortLabel(s: Sort): String = stringResource(
    when (s) {
        Sort.Title -> R.string.sort_title
        Sort.TitleDesc -> R.string.sort_title_desc
        Sort.Recent -> R.string.sort_recent
        Sort.Oldest -> R.string.sort_oldest
        Sort.YearDesc -> R.string.sort_year_desc
        Sort.Year -> R.string.sort_year
        Sort.SizeDesc -> R.string.sort_size
        Sort.EpisodesDesc -> R.string.sort_episodes
    },
)

@Composable
private fun watchLabel(w: Watch): String = stringResource(
    when (w) {
        Watch.Unwatched -> R.string.watch_unwatched
        Watch.InProgress -> R.string.watch_in_progress
        Watch.Watched -> R.string.progress_done
    },
)

@Composable
private fun formLabel(s: Form): String = stringResource(if (s == Form.Series) R.string.form_series else R.string.form_single)

@Composable
private fun addedLabel(days: Int): String = stringResource(
    when (days) {
        1 -> R.string.added_today
        7 -> R.string.added_week
        30 -> R.string.added_month
        else -> R.string.added_quarter
    },
)

/**
 * A title: the hero, the main action on arrival, then its episodes or
 * volumes as rows (thumbnail, label, state) and the facts.
 */
@Composable
fun TitleScreen(state: AppState, vm: AppViewModel, key: String, onDownload: (Item) -> Unit) {
    val l = LocalLain.current
    val title = state.titles.firstOrNull { it.key == key }
    if (title == null) {
        Centered { Text(stringResource(R.string.err_not_found), color = l.muted) }
        return
    }
    LaunchedEffect(key) { vm.loadProgress(title.items.map { it.id }) }
    val next = nextUp(title, state.progress)
    val watched = title.items.count { state.progress[it.id]?.completed == true }
    val series = title.items.size > 1
    LazyColumn(contentPadding = PaddingValues(bottom = 40.dp)) {
        item {
            Box(Modifier.fillMaxWidth().height(272.dp)) {
                Art(title.art(), Modifier.fillMaxSize())
                Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0f to l.bg.copy(alpha = 0.25f), 0.5f to l.bg.copy(alpha = 0.55f), 1f to l.bg)))
                Box(Modifier.padding(6.dp)) { IconBtn(Ic.ArrowLeft, "Back", { vm.back() }, tint = l.text) }
                Column(Modifier.align(Alignment.BottomStart).padding(horizontal = GUTTER).padding(bottom = 20.dp)) {
                    Mono(
                        listOfNotNull(
                            title.year.takeIf { it > 0 }?.toString(),
                            if (series) stringResource(R.string.format_series) else title.kind,
                            if (series) titleSubtitle(title) else null,
                        ).joinToString(" · "),
                        color = l.accent,
                    )
                    Text(
                        title.title.uppercase(),
                        Modifier.padding(top = 12.dp),
                        color = l.text, fontSize = 30.sp, lineHeight = 32.sp, fontWeight = FontWeight.Black, letterSpacing = (-1.1).sp,
                        maxLines = 3, overflow = TextOverflow.Ellipsis,
                    )
                    state.genresOf(title).takeIf { it.isNotEmpty() }?.let { g ->
                        Mono(g.take(4).joinToString("  /  "), Modifier.padding(top = 12.dp), color = l.text.copy(alpha = 0.75f), size = 11)
                    }
                }
            }
        }
        state.synopsisOf(title).takeIf { it.isNotEmpty() }?.let { text ->
            item {
                var open by remember { mutableStateOf(false) }
                Text(
                    text,
                    Modifier.padding(horizontal = GUTTER).padding(bottom = 16.dp).clickable { open = !open },
                    color = l.text.copy(alpha = 0.7f), fontSize = 14.sp, lineHeight = 22.sp, maxLines = if (open) 40 else 3, overflow = TextOverflow.Ellipsis,
                )
            }
        }
        item {
            Column(Modifier.padding(horizontal = GUTTER)) {
                val resume = state.progress[next.id]?.takeIf { !it.completed && it.positionSec > 0 }
                WButton(
                    when {
                        title.reading && resume != null -> stringResource(R.string.continue_page, resume.positionSec.toInt())
                        title.reading -> stringResource(R.string.action_read)
                        resume != null -> stringResource(R.string.resume_from, clock(resume.positionSec))
                        else -> stringResource(R.string.action_play)
                    } + if (series && next.episode > 0) " · " + episodeLabel(next.season, next.episode, title.reading) else "",
                    { vm.open(next) },
                    Modifier.fillMaxWidth(),
                    icon = Ic.Play,
                    large = true,
                )
                Text(
                    when {
                        watched == title.items.size -> stringResource(R.string.progress_done)
                        watched > 0 || resume != null -> stringResource(R.string.watched_of, watched, title.items.size)
                        else -> stringResource(R.string.watch_unwatched)
                    },
                    Modifier.fillMaxWidth().padding(top = 8.dp),
                    color = l.muted, fontSize = 12.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                )
                if (!series) {
                    Row(Modifier.padding(top = 10.dp)) {
                        WButton(stringResource(R.string.action_download), { onDownload(next) }, kind = Btn.Ghost, icon = Ic.Download)
                    }
                }
            }
        }
        if (series) {
            item {
                Text(
                    stringResource(if (title.reading) R.string.volumes else R.string.episodes),
                    Modifier.padding(start = GUTTER, top = 22.dp, bottom = 10.dp),
                    color = l.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.3).sp,
                )
            }
            items(title.items, key = { it.id }) { item -> EpisodeRow(item, title, state, vm, onDownload) }
        }
        item {
            Column(
                Modifier.padding(horizontal = GUTTER).padding(top = 22.dp).fillMaxWidth().clip(l.panelShape).border(1.dp, l.line.copy(alpha = 0.7f), l.panelShape)
                    .background(l.surface.copy(alpha = 0.3f)).padding(16.dp),
            ) {
                Mono(stringResource(R.string.information))
                Gap(8.dp)
                Fact(stringResource(R.string.fact_format), if (series) stringResource(R.string.format_series) else title.kind)
                if (title.year > 0) Fact(stringResource(R.string.fact_year), title.year.toString())
                if (series) Fact(stringResource(if (title.reading) R.string.volumes else R.string.episodes), title.items.size.toString())
                state.libraries.firstOrNull { it.id == title.libraryId }?.let { Fact(stringResource(R.string.nav_library), it.name) }
                Fact(stringResource(R.string.fact_size), bytes(title.items.sumOf { it.size }))
                Fact(stringResource(if (title.reading) R.string.fact_read else R.string.fact_watched), stringResource(R.string.n_of_m, watched, title.items.size))
            }
        }
    }
}

@Composable
private fun Fact(label: String, value: String) {
    val l = LocalLain.current
    Column {
        Box(Modifier.fillMaxWidth().height(1.dp).background(l.line.copy(alpha = 0.6f)))
        Row(Modifier.fillMaxWidth().padding(vertical = 9.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(label.uppercase(), Modifier.weight(1f), color = l.muted, fontSize = 12.sp, letterSpacing = 0.6.sp)
            Text(value, color = l.text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** An episode or volume as a row: six fit a screen. */
@Composable
private fun EpisodeRow(item: Item, title: Title, state: AppState, vm: AppViewModel, onDownload: (Item) -> Unit) {
    val l = LocalLain.current
    val p = state.progress[item.id]
    Row(
        Modifier.padding(horizontal = GUTTER, vertical = 4.dp).fillMaxWidth().clip(l.cardShape).border(1.dp, l.line.copy(alpha = 0.6f), l.cardShape)
            .background(l.surface.copy(alpha = 0.3f)).clickable { vm.open(item) },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(if (title.reading) 64.dp else 144.dp).aspectRatio(if (title.reading) 3f / 4f else 16f / 9f)) {
            Art(item.id, Modifier.fillMaxSize())
            val f = if (p?.completed == true) 0f else fraction(p)
            if (f > 0f) {
                Box(Modifier.align(Alignment.BottomStart).fillMaxWidth().height(4.dp).background(Color.Black.copy(alpha = 0.5f))) {
                    Box(Modifier.fillMaxWidth(f).height(4.dp).background(l.accent))
                }
            }
        }
        Column(Modifier.weight(1f).padding(horizontal = 14.dp)) {
            Mono(
                if (item.episode > 0) stringResource(if (title.reading) R.string.volume_n else R.string.episode_n, item.episode.toInt()) +
                    (if (item.season > 0 && !title.reading) " · S${item.season}" else "") else item.title,
                color = l.accent,
            )
            Text(
                when {
                    p?.completed == true -> stringResource(R.string.progress_done)
                    p != null && p.positionSec > 0 && title.reading -> stringResource(R.string.progress_page, p.positionSec.toInt(), p.durationSec.toInt())
                    p != null && p.positionSec > 0 -> stringResource(R.string.progress_at, clock(p.positionSec))
                    else -> bytes(item.size)
                },
                Modifier.padding(top = 5.dp),
                color = l.muted, fontSize = 12.sp, maxLines = 1,
            )
        }
        IconBtn(Ic.Download, stringResource(R.string.action_download), { onDownload(item) })
    }
}

/** Search: one field, results as cards by shape. */
@Composable
fun SearchScreen(state: AppState, vm: AppViewModel) {
    val l = LocalLain.current
    Column(Modifier.fillMaxSize()) {
        Text(stringResource(R.string.nav_search), Modifier.padding(start = GUTTER, top = 18.dp), color = l.text, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.4).sp)
        SearchField(state.query, { vm.search(it) }, stringResource(R.string.search_hint), Modifier.padding(horizontal = GUTTER).padding(top = 12.dp).fillMaxWidth())
        when {
            state.query.isBlank() -> Column(Modifier.fillMaxWidth().padding(top = 64.dp, start = 40.dp, end = 40.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Box(Modifier.size(56.dp).clip(CircleShape).border(1.dp, l.line, CircleShape), contentAlignment = Alignment.Center) { Icon(Ic.Search, null, tint = l.muted, modifier = Modifier.size(22.dp)) }
                Text(stringResource(R.string.search_title), Modifier.padding(top = 18.dp), color = l.text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
                Text(stringResource(R.string.search_body), Modifier.padding(top = 6.dp), color = l.muted, fontSize = 14.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            }
            state.searching && state.results.isEmpty() -> Mono(stringResource(R.string.working), Modifier.padding(GUTTER))
            state.results.isEmpty() -> Text(stringResource(R.string.search_none), Modifier.padding(GUTTER), color = l.muted, fontSize = 14.sp)
            else -> {
                Text(pluralStringResource(R.plurals.results_for, state.results.size, state.results.size, state.query.trim()), Modifier.padding(start = GUTTER, top = 14.dp), color = l.muted, fontSize = 14.sp)
                // Shapes are read in composition, never in the span lambda (see LibraryBrowser).
                val results = state.results
                val wide = results.map { Shapes.landscape(it.id, !it.reading()) }
                LazyVerticalGrid(
                    columns = GridCells.Fixed(6),
                    contentPadding = PaddingValues(start = GUTTER, end = GUTTER, top = 14.dp, bottom = 32.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalArrangement = Arrangement.spacedBy(20.dp),
                ) {
                    items(results.size, key = { results[it].id }, span = { GridItemSpan(if (wide[it]) 6 else 3) }) { i ->
                        val item = results[i]
                        ShapeCard(
                            artId = item.id,
                            title = item.title,
                            subtitle = episodeText(item),
                            onClick = { vm.go(Screen.TitleDetail(titleKey(item))) },
                            landscape = wide[i],
                        )
                    }
                }
            }
        }
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


/**
 * Unlocking private libraries for this session: the account's library
 * PIN, or its password. Nothing here names a library or says how many
 * there are.
 */
@Composable
fun UnlockSheet(vm: AppViewModel, onClose: () -> Unit) {
    val l = LocalLain.current
    val scope = rememberCoroutineScope()
    var usePin by remember { mutableStateOf(true) }
    var secret by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<Throwable?>(null) }
    fun submit() {
        if (secret.isEmpty() || busy) return
        busy = true
        scope.launch {
            error = vm.unlock(secret, usePin)
            busy = false
            if (error == null) onClose() else secret = ""
        }
    }
    Sheet(stringResource(R.string.unlock_title), onClose) {
        Text(stringResource(R.string.unlock_body), color = l.muted, fontSize = 14.sp, lineHeight = 20.sp)
        Gap(16.dp)
        Text(stringResource(if (usePin) R.string.unlock_pin else R.string.login_password), color = l.muted, fontSize = 14.sp, fontWeight = FontWeight.Medium)
        Gap(6.dp)
        SecretField(secret, { secret = it; error = null }, numeric = usePin, onDone = { submit() }, modifier = Modifier.fillMaxWidth())
        error?.let { ErrorLine(it, Modifier.padding(top = 8.dp)) }
        Gap(16.dp)
        WButton(stringResource(if (busy) R.string.working else R.string.unlock_go), { submit() }, Modifier.fillMaxWidth(), enabled = secret.isNotEmpty() && !busy, large = true)
        Text(
            stringResource(if (usePin) R.string.unlock_use_password else R.string.unlock_use_pin),
            Modifier.fillMaxWidth().clickable { usePin = !usePin; secret = ""; error = null }.padding(vertical = 14.dp),
            color = l.muted, fontSize = 14.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center,
        )
    }
}
