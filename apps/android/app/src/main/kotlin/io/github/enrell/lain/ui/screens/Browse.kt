package io.github.enrell.lain.ui.screens

import io.github.enrell.lain.domain.favoriteTitles
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.DisposableEffect
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.ColumnScope
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
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.shape.RoundedCornerShape
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
import io.github.enrell.lain.domain.goOnWith
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
    t.items.size > 1 && t.reading -> pluralStringResource(R.plurals.count_volumes, t.items.size, t.items.size)
    t.items.size > 1 -> pluralStringResource(R.plurals.count_episodes, t.items.size, t.items.size)
    t.year > 0 -> t.year.toString()
    else -> ""
}

/** A title's card at rail height: as wide as its picture's shape. */
@Composable
private fun RailCard(t: Title, vm: AppViewModel, height: Dp = RAIL_H, seen: Float = 0f) {
    val wide = t.wide()
    ShapeCard(
        artId = t.art(),
        title = t.title,
        subtitle = titleSubtitle(t),
        onClick = { vm.go(Screen.TitleDetail(t.key)) },
        modifier = Modifier.width(height * (if (wide) Shapes.LANDSCAPE else Shapes.PORTRAIT)),
        landscape = wide,
        progress = seen,
        badge = null,
    )
}

/** How far into a title you are: its finished parts plus the part of the one in progress, of the whole. */
@Composable
private fun progressSubtitle(t: Title, progress: Map<String, io.github.enrell.lain.core.ProgressInfo>): String? {
    val done = t.items.count { progress[it.id]?.completed == true }
    val currentItem = t.items.firstOrNull { item -> progress[item.id]?.let { !it.completed && it.positionSec > 0 } == true }
    val current = currentItem?.let { progress[it.id] }
    return when {
        // Finished: said, not left to the count of parts.
        t.items.size > 1 && done == t.items.size ->
            stringResource(if (t.reading) R.string.all_read else R.string.all_watched, t.items.size)
        // A book opened but none finished: where in it, not "0 of 3 read".
        t.items.size > 1 && t.reading && done == 0 && currentItem != null && current != null && current.durationSec > 0 ->
            rowLabelShort(currentItem, true) + " · " + stringResource(R.string.progress_page, current.positionSec.toInt().coerceAtLeast(1), current.durationSec.toInt())
        // An episode begun and none finished: which one and what is left, not "0 of 12 watched".
        t.items.size > 1 && done == 0 && currentItem != null && current != null && current.durationSec > 0 ->
            rowLabelShort(currentItem, false) + " · " + stringResource(R.string.time_left, length((current.durationSec - current.positionSec).coerceAtLeast(1.0)))
        t.items.size > 1 && (done > 0 || current != null) && done < t.items.size ->
            stringResource(if (t.reading) R.string.read_of else R.string.watched_of, done, t.items.size)
        t.items.size == 1 && current != null && current.durationSec > 0 ->
            if (t.reading) stringResource(R.string.progress_page, current.positionSec.toInt().coerceAtLeast(1), current.durationSec.toInt())
            else stringResource(R.string.time_left, length((current.durationSec - current.positionSec).coerceAtLeast(1.0)))
        else -> null
    }
}

private fun titleFraction(t: Title, progress: Map<String, io.github.enrell.lain.core.ProgressInfo>): Float {
    if (t.items.isEmpty()) return 0f
    val sum = t.items.sumOf { item ->
        val p = progress[item.id]
        when {
            p == null -> 0.0
            p.completed -> 1.0
            p.durationSec > 0 -> (p.positionSec / p.durationSec).coerceIn(0.0, 1.0)
            else -> 0.0
        }
    }
    return (sum / t.items.size).toFloat().let { if (it in 0.0001f..0.03f) 0.03f else it }
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
        if (action != null) Text(action, color = l.muted, fontSize = 13.sp)
    }
}

/** Home: a title-first hero, then Continue, Recently added and one rail per library. */
@Composable
fun HomeScreen(state: AppState, vm: AppViewModel) {
    val l = LocalLain.current
    val rails = remember(state.titles, state.continues, state.progress, state.libraries, state.favorites) {
        val goOn = goOnWith(state.titles, state.continues, state.continues.associateBy { it.itemId } + state.progress)
        buildRails(state.titles, goOn, state.items, state.libraries, favoriteTitles(state.titles, state.favorites.orEmpty()))
    }
    if (rails.isEmpty()) {
        if (state.busy && state.error == null) {
            HomeSkeleton()
        } else {
            Box(Modifier.fillMaxSize().padding(GUTTER), contentAlignment = Alignment.Center) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    EmptyState(Ic.Library, stringResource(R.string.home_empty_title), stringResource(R.string.home_empty)) {
                        WButton(stringResource(R.string.action_refresh), { vm.refresh() }, kind = Btn.Secondary, icon = Ic.Refresh)
                    }
                    ErrorLine(state.error, Modifier.padding(top = 12.dp))
                }
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
    val resumeHero = remember(rails, hero) { (rails.firstOrNull()?.cards?.firstOrNull() as? Card.Resume)?.takeIf { hero != null && titleKey(it.item) == hero.key } }
    Refreshable(state.busy, { vm.refresh() }) {
    // Room under the last rail, so its captions end clear of the bar.
    // Back from a title lands where Home was left, not at its top.
    val homeList = rememberListMemory("home")
    LazyColumn(state = homeList, contentPadding = PaddingValues(bottom = 56.dp)) {
        if (hero != null) {
            item {
                HeroFrame(hero.art(), (LocalConfiguration.current.screenHeightDp * 0.44f).dp) {
                    Mono(
                        stringResource(if (resumeHero == null) R.string.hero_label else if (hero.reading) R.string.hero_continue_reading else R.string.hero_continue),
                        color = l.accent,
                    )
                    Text(
                        hero.title.uppercase(),
                        Modifier.padding(top = 10.dp),
                        color = l.text, fontSize = 34.sp, lineHeight = 34.sp, fontWeight = FontWeight.Black, letterSpacing = (-1.5).sp,
                        maxLines = 3, overflow = TextOverflow.Ellipsis,
                    )
                    // What it is, in words: the episode you are on and what is left of it, or the
                    // size of the thing; then up to two genres. Tokens drop before anything is cut.
                    // A next part not begun yet (the one after the last finished) is "up next", not time left.
                    val fresh = resumeHero != null && resumeHero.progress.positionSec < 1
                    val left = resumeHero?.progress?.takeIf { !hero.reading && !fresh }?.let { p -> (p.durationSec - p.positionSec).takeIf { it > 0 && p.durationSec > 0 } }
                    val facts = listOfNotNull(
                        resumeHero?.item?.takeIf { it.episode > 0 || (hero.reading && it.season > 0) }?.let { rowLabelShort(it, hero.reading) },
                        resumeHero?.takeIf { hero.reading && it.progress.durationSec > 0 }?.let { stringResource(R.string.progress_page, it.progress.positionSec.toInt().coerceAtLeast(1), it.progress.durationSec.toInt()) },
                        left?.let { stringResource(R.string.time_left, length(it)) },
                        if (fresh && !hero.reading) stringResource(R.string.up_next) else null,
                        if (resumeHero == null && hero.items.size > 1) titleSubtitle(hero) else null,
                        hero.year.takeIf { it > 0 }?.toString(),
                    ) + state.genresOf(hero).take(2)
                    Text(facts.joinToString("  ·  "), Modifier.padding(top = 10.dp), color = l.text.copy(alpha = 0.8f), fontSize = 13.sp, lineHeight = 19.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    resumeHero?.progress?.let { p ->
                        val f = fraction(p)
                        if (f > 0f) {
                            Box(Modifier.padding(top = 12.dp).fillMaxWidth(0.6f).height(4.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.18f))) {
                                Box(Modifier.fillMaxWidth(f).height(4.dp).clip(CircleShape).background(l.accent))
                            }
                        }
                    }
                    Row(Modifier.padding(top = 18.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Row(
                            Modifier.height(46.dp).clip(CircleShape).background(Color.White).clickable { vm.open(resumeHero?.item ?: nextUp(hero, state.progress)) }.padding(horizontal = 24.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Icon(Ic.Play, null, tint = Color.Black, modifier = Modifier.size(14.dp))
                            Text(
                                when {
                                    resumeHero != null && hero.reading && resumeHero.progress.positionSec >= 1 -> stringResource(R.string.continue_page_short, resumeHero.progress.positionSec.toInt())
                                    hero.reading -> stringResource(R.string.action_read)
                                    fresh && resumeHero != null && resumeHero.item.episode > 0 -> stringResource(R.string.action_play) + " · " + stringResource(R.string.episode_n, resumeHero.item.episode.toInt())
                                    resumeHero != null && resumeHero.item.episode > 0 -> stringResource(R.string.action_resume_short) + " · " + stringResource(R.string.episode_n, resumeHero.item.episode.toInt())
                                    resumeHero != null -> stringResource(R.string.action_resume_short)
                                    hero.reading -> stringResource(R.string.action_read)
                                    else -> stringResource(R.string.action_play)
                                },
                                color = Color.Black, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, maxLines = 1,
                            )
                        }
                        Row(
                            Modifier.height(46.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.35f)).border(1.dp, Color.White.copy(alpha = 0.2f), CircleShape)
                                .clickable { vm.go(Screen.TitleDetail(hero.key)) }.padding(horizontal = 20.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Icon(Ic.Info, null, tint = l.text, modifier = Modifier.size(16.dp))
                            Text(stringResource(R.string.action_details), color = l.text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, softWrap = false)
                        }
                    }
                }
            }
        }
        if (state.fromCache) item { OfflineNote(state) }
        // What the hero resumes is not repeated as the first card of Continue.
        val shown = rails.map { r ->
            if ((r.label == RailLabel.Continue || r.label == RailLabel.ContinueReading) && resumeHero != null) r.copy(cards = r.cards.filterNot { c -> c is Card.Resume && c.item.id == resumeHero.item.id }) else r
        }.filter { it.cards.isNotEmpty() }
        items(shown) { rail ->
            Column(Modifier.padding(top = 26.dp)) {
                val lib = (rail.label as? RailLabel.Library)
                val libId = lib?.let { n -> state.libraries.firstOrNull { it.name == n.name }?.id }
                RailHeader(
                    when (val lab = rail.label) {
                        RailLabel.Continue -> stringResource(R.string.rail_continue)
                        RailLabel.ContinueReading -> stringResource(R.string.rail_continue_reading)
                        RailLabel.Recent -> stringResource(R.string.rail_recent)
                        RailLabel.MyList -> stringResource(R.string.rail_my_list)
                        is RailLabel.Library -> lab.name
                    },
                    null,
                    // A rail is a sample: a library's opens the library, the favourites' their own page.
                    libId?.let { id -> { openLibrary(vm, id) } } ?: if (rail.label == RailLabel.MyList) ({ vm.go(Screen.LibraryDetail(io.github.enrell.lain.ui.FAVORITES)) }) else null,
                )
                val lone = rail.cards.singleOrNull() as? Card.Resume
                if (lone != null && lone.item.reading() && lone.progress.durationSec > 0) {
                    // One book under way: the cover with its place beside it, across the row. As a
                    // rail of one, a narrow cover sat at the left of a row of nothing and broke its
                    // name over two lines.
                    LoneBook(lone, { vm.open(lone.item) })
                } else
                LazyRow(Modifier.padding(top = 8.dp), contentPadding = PaddingValues(horizontal = GUTTER), horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                    items(rail.cards.take(20)) { card ->
                        when (card) {
                            is Card.Resume -> ShapeCard(
                                artId = card.item.id,
                                title = card.item.title,
                                subtitle = if (card.item.reading() && card.progress.durationSec > 0) {
                                    // Which book as well as which page: "Page 3 of 8" alone fits any volume of the title.
                                    listOfNotNull(
                                        card.item.takeIf { it.episode > 0 || it.season > 0 }?.let { rowLabelShort(it, true) },
                                        stringResource(R.string.progress_page, card.progress.positionSec.toInt().coerceAtLeast(1), card.progress.durationSec.toInt()),
                                    ).joinToString(" · ")
                                } else {
                                    listOfNotNull(
                                        card.item.takeIf { it.episode > 0 }?.let { episodeText(it) },
                                        if (card.progress.positionSec < 1) stringResource(R.string.up_next)
                                        else (card.progress.durationSec - card.progress.positionSec).takeIf { it > 0 }?.let { stringResource(R.string.time_left, length(it)) },
                                    ).joinToString(" · ").ifEmpty { episodeText(card.item) }
                                },
                                onClick = { vm.open(card.item) },
                                // A book resumes from its cover, at a cover's shape; a video from a still.
                                modifier = Modifier.width(RAIL_H * (if (card.item.reading()) Shapes.PORTRAIT else Shapes.LANDSCAPE)),
                                landscape = !card.item.reading(),
                                progress = fraction(card.progress),
                            )
                            is Card.Show -> RailCard(card.title, vm, seen = titleFraction(card.title, allProgressOf(state)))
                        }
                    }
                }
            }
        }
    }
    }
}

/** A lone book in progress on Home: its cover, its name, where you are and how far. */
@Composable
private fun LoneBook(card: Card.Resume, onOpen: () -> Unit) {
    val l = LocalLain.current
    val p = card.progress
    Row(
        Modifier.padding(top = 8.dp).padding(horizontal = GUTTER).fillMaxWidth().clip(l.cardShape).background(l.tonalLow)
            .clickable(onClickLabel = card.item.title, onClick = onOpen).padding(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(RAIL_H * Shapes.PORTRAIT * 0.8f).aspectRatio(Shapes.PORTRAIT).clip(l.cardShape).background(l.surface)) {
            Art(card.item.id, Modifier.fillMaxSize(), label = card.item.title)
        }
        Column(Modifier.weight(1f).padding(start = 16.dp, end = 6.dp)) {
            Text(card.item.title, color = l.text, fontSize = 17.sp, lineHeight = 22.sp, fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(
                listOfNotNull(
                    card.item.takeIf { it.episode > 0 || it.season > 0 }?.let { rowLabelShort(it, true) },
                    stringResource(R.string.progress_page, p.positionSec.toInt().coerceAtLeast(1), p.durationSec.toInt()),
                ).joinToString(" · "),
                Modifier.padding(top = 4.dp), color = l.muted, fontSize = 13.sp, maxLines = 1,
            )
            Box(Modifier.padding(top = 12.dp).fillMaxWidth().height(4.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.12f))) {
                Box(Modifier.fillMaxWidth(fraction(p).coerceIn(0.02f, 1f)).height(4.dp).clip(CircleShape).background(l.accent))
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
                val ctx = LocalContext.current
                val lockedText = stringResource(R.string.private_locked)
                // Open is a different picture and a different colour, with its name beside it.
                Row(
                    Modifier.height(40.dp).clip(CircleShape).background(if (locked) l.tonal else l.accent.copy(alpha = 0.16f)).border(1.dp, if (locked) Color.Transparent else l.accent.copy(alpha = 0.6f), CircleShape)
                        .clickable(onClickLabel = stringResource(if (locked) R.string.unlock_title else R.string.lock_again)) {
                            if (locked) unlocking = true else {
                                vm.lockAll()
                                android.widget.Toast.makeText(ctx, lockedText, android.widget.Toast.LENGTH_SHORT).show()
                            }
                        }.padding(horizontal = 14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(7.dp),
                ) {
                    Icon(if (locked) Ic.Lock else Ic.LockOpen, null, tint = if (locked) l.muted else l.accent, modifier = Modifier.size(16.dp))
                    Text(stringResource(if (locked) R.string.unlock_short else R.string.lock_short), color = if (locked) l.muted else l.text, fontSize = 13.sp, fontWeight = FontWeight.Medium)
                }
            }
        }
        if (unlocking) UnlockSheet(vm) { unlocking = false }
        if (libraries.size > 1) {
            Segmented(
                listOf(true to stringResource(R.string.view_by_library), false to stringResource(R.string.view_all_titles)),
                shelves,
                { shelves = it; prefs.libraryShelves = it },
                Modifier.padding(horizontal = GUTTER).padding(top = 12.dp),
            )
        }
        if (state.fromCache) OfflineNote(state)
        if (titles.isEmpty()) {
            if (state.busy) {
                Column(Modifier.padding(GUTTER), verticalArrangement = Arrangement.spacedBy(18.dp)) {
                    repeat(3) { Skeleton(Modifier.fillMaxWidth().height(RAIL_H)) }
                }
            } else {
                Box(Modifier.padding(GUTTER)) {
                    EmptyState(Ic.Library, stringResource(R.string.library_empty_title), stringResource(R.string.library_empty)) {
                        WButton(stringResource(R.string.action_refresh), { vm.refresh() }, kind = Btn.Secondary, icon = Ic.Refresh)
                    }
                }
            }
            return
        }
        if (byShelf) {
            Refreshable(state.busy, { vm.refresh() }) {
            // Private libraries come first once unlocked: what the PIN opened is on screen.
            val ordered = remember(libraries) { libraries.sortedByDescending { it.private } }
            val shelvesState = rememberListMemory("library-root")
            val anyOpen = state.libraries.any { it.private && !it.locked }
            LaunchedEffect(anyOpen) { if (anyOpen) shelvesState.animateScrollToItem(0) }
            LazyColumn(state = shelvesState, contentPadding = PaddingValues(top = 12.dp, bottom = 56.dp)) {
                items(ordered, key = { it.id }) { lib ->
                    val mine = remember(titles, lib.id) { titles.filter { it.libraryId == lib.id }.sortedByDescending { it.latest } }
                    Column(Modifier.padding(top = 20.dp)) {
                        RailHeader(lib.name + if (lib.private) "  ·  " + stringResource(R.string.private_badge) else "", pluralStringResource(R.plurals.count_titles, mine.size, mine.size)) {
                            openLibrary(vm, lib.id)
                        }
                        LazyRow(Modifier.padding(top = 8.dp), contentPadding = PaddingValues(horizontal = GUTTER), horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                            items(mine.take(14), key = { it.key }) { RailCard(it, vm, seen = titleFraction(it, allProgressOf(state))) }
                            if (mine.size > 14) {
                                item {
                                    Box(
                                        Modifier.height(RAIL_H).width(112.dp).clip(l.cardShape).background(l.tonal).clickable { openLibrary(vm, lib.id) },
                                        contentAlignment = Alignment.Center,
                                    ) { Text(stringResource(R.string.see_all), color = l.muted, fontSize = 14.sp) }
                                }
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
    // The favourites have a page like a library's: searched, ordered and narrowed the same way.
    val favorites = id == io.github.enrell.lain.ui.FAVORITES
    val titles = remember(state.titles, state.favorites, id) {
        if (favorites) favoriteTitles(state.titles, state.favorites.orEmpty()) else state.titles.filter { it.libraryId == id }
    }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(start = 8.dp, end = GUTTER, top = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            IconBtn(Ic.ArrowLeft, stringResource(R.string.nav_library), { vm.back() }, tint = l.text)
            Text(if (favorites) stringResource(R.string.rail_my_list) else lib?.name ?: "", Modifier.padding(start = 4.dp), color = l.text, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.4).sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Box(Modifier.padding(top = 8.dp)) { LibraryBrowser(titles, state, vm, searchHint = stringResource(R.string.search_library), sortScope = id) }
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
fun LibraryBrowser(titles: List<Title>, state: AppState, vm: AppViewModel, searchHint: String, sortScope: String = "") {
    val l = LocalLain.current
    val prefs = LocalContext.current.graph.prefs
    val anySeries = remember(titles) { titles.any { it.items.size > 1 } }
    // An order that cannot apply here (most episodes among single films) is not kept.
    var filters by remember { mutableStateOf(Filters(sort = Sort.of(prefs.sortOf(sortScope)).takeIf { it != Sort.EpisodesDesc || anySeries } ?: Sort.Title)) }
    // What a closed sheet left unapplied, offered back until it is applied or let go:
    // a bar that left by itself took the pending choices with it.
    var unapplied by remember { mutableStateOf<Filters?>(null) }
    var dense by remember { mutableStateOf(prefs.smallTiles) }
    var sheet by remember { mutableStateOf(false) }
    val now = remember { System.currentTimeMillis() / 1000 }
    val progress = remember(state.continues, state.progress) { state.allProgress() }
    val genresOf: (Title) -> List<String> = remember(state.overlays) { { t: Title -> state.genresOf(t) } }
    val results = remember(titles, filters, progress, state.overlays) { applyFilters(titles, filters, progress, now, genresOf) }
    val grid = rememberGridMemory("grid-" + (titles.firstOrNull()?.libraryId ?: "") + "-" + titles.size)
    val scope = rememberCoroutineScope()
    val far by remember { derivedStateOf { grid.firstVisibleItemIndex > 10 } }
    // A new result set starts at its beginning (not the one a screen comes back to).
    var first by remember { mutableStateOf(true) }
    LaunchedEffect(filters) { if (first) first = false else grid.scrollToItem(0) }

    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Row(Modifier.padding(horizontal = GUTTER), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                SearchField(filters.q, { filters = filters.copy(q = it) }, searchHint, Modifier.weight(1f))
                val narrowing = filters.active > 0 || filters.sort != Sort.Title
                Row(
                    Modifier.height(44.dp).clip(l.buttonShape).background(if (narrowing) l.accent.copy(alpha = 0.16f) else l.tonal).border(1.dp, if (narrowing) l.accent.copy(alpha = 0.7f) else Color.Transparent, l.buttonShape)
                        .clickable(onClickLabel = stringResource(R.string.filters)) { sheet = true }.padding(horizontal = 12.dp),
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    // How many are set, as a badge on the icon: spelled after the label ("· 1") it
                    // widened the button and the search field beside it shrank under the thumb.
                    val set = filters.active + if (filters.sort != Sort.Title) 1 else 0
                    Box {
                        Icon(Ic.Sliders, null, tint = if (narrowing) l.text else l.muted, modifier = Modifier.size(18.dp))
                        if (set > 0) {
                            Box(
                                Modifier.align(Alignment.TopEnd).offset(x = 7.dp, y = (-7).dp).size(16.dp).clip(CircleShape).background(l.accent),
                                contentAlignment = Alignment.Center,
                            ) { Text(set.toString(), color = l.onAccent, fontSize = 10.sp, fontWeight = FontWeight.Bold, lineHeight = 10.sp, maxLines = 1) }
                        }
                    }
                    Text(stringResource(R.string.filters), color = if (narrowing) l.text else l.muted, fontSize = 14.sp, fontWeight = FontWeight.Medium, maxLines = 1)
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
                if (filters.sort != Sort.Title) {
                    FilterChip(stringResource(R.string.sorted_by, sortLabel(filters.sort))) { filters = filters.copy(sort = Sort.Title); prefs.setSortOf(sortScope, Sort.Title.id) }
                }
                for (g in filters.genres) FilterChip(g) { filters = filters.copy(genres = filters.genres - g) }
                filters.watch?.let { w -> FilterChip(watchLabel(w)) { filters = filters.copy(watch = null) } }
                filters.form?.let { s -> FilterChip(formLabel(s)) { filters = filters.copy(form = null) } }
                if (filters.kind.isNotEmpty()) FilterChip(filters.kind) { filters = filters.copy(kind = "") }
                if (filters.added > 0) FilterChip(addedLabel(filters.added)) { filters = filters.copy(added = 0) }
                if (filters.narrowed) {
                    Text(stringResource(R.string.clear), Modifier.clip(CircleShape).clickable { filters = filters.cleared(); prefs.setSortOf(sortScope, Sort.Title.id) }.padding(horizontal = 8.dp, vertical = 4.dp), color = l.muted, fontSize = 12.sp)
                }
            }
            if (results.isEmpty()) {
                Column(Modifier.fillMaxWidth().padding(top = 56.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        if (filters.q.isNotBlank()) stringResource(R.string.nothing_matches, filters.q.trim()) else stringResource(R.string.nothing_passes),
                        color = l.text, fontSize = 14.sp,
                    )
                    Gap(16.dp)
                    WButton(stringResource(R.string.clear_all), { filters = filters.cleared(); prefs.setSortOf(sortScope, Sort.Title.id) }, kind = Btn.Secondary)
                }
            } else {
                // The shapes are read here, in composition, and handed to the grid
                // as a plain list: a lazy grid caches its rows from the spans, so
                // a span that changed behind its back (a cover that loaded and
                // turned a card from landscape to portrait) left an item in two
                // rows and crashed the layout. A new list rebuilds the rows.
                val wide = results.map { it.wide() }
                LazyVerticalGrid(
                    columns = EvenColumns,
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
                        val seen = titleFraction(t, progress)
                        ShapeCard(
                            artId = t.art(),
                            title = t.title,
                            // A title under way says how far it is, where an untouched one says what it is.
                            subtitle = progressSubtitle(t, progress) ?: titleSubtitle(t),
                            onClick = { vm.go(Screen.TitleDetail(t.key)) },
                            landscape = wide[i],
                            progress = seen,
                            badge = null,
                        )
                    }
                }
            }
        }
        unapplied?.let { left ->
            Row(
                Modifier.align(Alignment.BottomCenter).padding(horizontal = GUTTER, vertical = 16.dp).fillMaxWidth().clip(l.buttonShape).background(l.raised).border(1.dp, l.line, l.buttonShape).padding(start = 14.dp, end = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(stringResource(R.string.filters_unapplied), Modifier.weight(1f).padding(vertical = 12.dp), color = l.text, fontSize = 14.sp)
                Text(
                    stringResource(R.string.apply),
                    Modifier.clip(l.buttonShape).clickable { filters = left; prefs.setSortOf(sortScope, left.sort.id); unapplied = null }.padding(horizontal = 12.dp, vertical = 12.dp),
                    color = l.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold,
                )
                IconBtn(Ic.Close, stringResource(R.string.discard_changes), { unapplied = null }, tint = l.muted)
            }
        }
        if (far && unapplied == null) {
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
        // The sheet edits a draft: "Show N titles" applies it, and closing the
        // sheet any other way (a drag, the scrim, Close) leaves the list as it was.
        var draft by remember { mutableStateOf(filters) }
        var draftDense by remember { mutableStateOf(dense) }
        val shown = remember(titles, draft, progress, state.overlays) { applyFilters(titles, draft, progress, now, genresOf).size }
        val watchCounts = countBy(titles, draft, progress, now, Axis.Watch, genresOf) { listOf(watchState(it, progress)) }
        val formCounts = countBy(titles, draft, progress, now, Axis.Form, genresOf) { listOf(if (it.items.size > 1) Form.Series else Form.Single) }
        val kindCounts = countBy(titles, draft, progress, now, Axis.Kind, genresOf) { t -> t.items.map { it.kind }.distinct() }
        // The genres keep their places while you pick: the list is the library's,
        // and only the counts follow the other choices.
        val genreNames = remember(titles, state.overlays) { genreFacets(titles, Filters(), progress, now, genresOf).map { it.first } }
        val genreCounts = remember(titles, draft, progress, state.overlays) { genreFacets(titles, draft, progress, now, genresOf).associate { it.first.lowercase() to it.second } }
        var allGenres by remember { mutableStateOf(false) }
        val kinds = remember(titles) { titles.flatMap { t -> t.items.map { it.kind } }.distinct().filter { it.isNotEmpty() }.sorted() }
        val changed = draft.active > 0 || draft.sort != Sort.Title
        Sheet(
            stringResource(R.string.filters),
            {
                if (draft != filters || draftDense != dense) unapplied = draft
                sheet = false
            },
            dragToClose = false,
            footer = {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    WButton(stringResource(R.string.clear), { draft = Filters(q = draft.q) }, Modifier.weight(1f), kind = Btn.Secondary, enabled = changed)
                    WButton(
                        pluralStringResource(R.plurals.show_titles, shown, shown),
                        {
                            filters = draft
                            dense = draftDense
                            prefs.setSortOf(sortScope, draft.sort.id)
                            unapplied = null
                            prefs.smallTiles = draftDense
                            sheet = false
                        },
                        Modifier.weight(1.4f),
                    )
                }
            },
        ) {
            SheetHeading(stringResource(R.string.sort))
            // Each order beside its reverse.
            val sorts = listOfNotNull(Sort.Title, Sort.TitleDesc, Sort.YearDesc, Sort.Year, Sort.Recent, if (anySeries) Sort.EpisodesDesc else null)
            for (pair in sorts.chunked(2)) {
                Row(Modifier.fillMaxWidth().padding(vertical = 3.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    for (s in pair) ChoiceTile(sortLabel(s), draft.sort == s, { draft = draft.copy(sort = s) }, Modifier.weight(1f))
                    if (pair.size == 1) Box(Modifier.weight(1f))
                }
            }
            // The short lists first, so nothing hides under a long one: the genres can run past the fold.
            // Each choice of one starts with "Any": a ring that was picked had no
            // visible way back to everything short of Clear, which also drops the genres.
            SheetHeading(stringResource(R.string.progress))
            OptionRow(stringResource(R.string.filter_any), remember(titles, draft, progress, state.overlays) { applyFilters(titles, draft.copy(watch = null), progress, now, genresOf).size }, draft.watch == null, { draft = draft.copy(watch = null) })
            for (w in Watch.entries) {
                OptionRow(watchLabel(w), watchCounts[w] ?: 0, draft.watch == w, { draft = draft.copy(watch = if (draft.watch == w) null else w) }, enabled = (watchCounts[w] ?: 0) > 0)
            }
            if (genreNames.isNotEmpty()) {
                SheetHeading(stringResource(R.string.genres))
                // Always there, so picking the first genre does not push the list down under the finger.
                Text(stringResource(R.string.genres_all_hint), Modifier.padding(bottom = 6.dp), color = l.muted, fontSize = 12.sp)
                for (g in if (allGenres) genreNames else genreNames.take(8)) {
                    val on = draft.genres.any { it.equals(g, ignoreCase = true) }
                    val n = genreCounts[g.lowercase()] ?: 0
                    OptionRow(g, n, on, { draft = draft.copy(genres = if (on) draft.genres.filterNot { it.equals(g, ignoreCase = true) } else draft.genres + g) }, enabled = n > 0, multi = true)
                }
                if (genreNames.size > 8) {
                    Text(
                        stringResource(if (allGenres) R.string.show_fewer else R.string.show_all_n, genreNames.size),
                        Modifier.bleed(12.dp).clip(l.buttonShape).clickable { allGenres = !allGenres }.padding(horizontal = 12.dp, vertical = 12.dp),
                        color = l.text, fontSize = 13.sp, fontWeight = FontWeight.Medium,
                    )
                }
            }
            if ((formCounts[Form.Series] ?: 0) > 0 && (formCounts[Form.Single] ?: 0) > 0) {
                SheetHeading(stringResource(R.string.form))
                OptionRow(stringResource(R.string.filter_any), remember(titles, draft, progress, state.overlays) { applyFilters(titles, draft.copy(form = null), progress, now, genresOf).size }, draft.form == null, { draft = draft.copy(form = null) })
                for (s in Form.entries) {
                    OptionRow(formLabel(s), formCounts[s] ?: 0, draft.form == s, { draft = draft.copy(form = if (draft.form == s) null else s) })
                }
            }
            if (kinds.size > 1) {
                SheetHeading(stringResource(R.string.kind))
                OptionRow(stringResource(R.string.filter_any), remember(titles, draft, progress, state.overlays) { applyFilters(titles, draft.copy(kind = ""), progress, now, genresOf).size }, draft.kind.isEmpty(), { draft = draft.copy(kind = "") })
                for (k in kinds) {
                    OptionRow(k, kindCounts[k] ?: 0, draft.kind == k, { draft = draft.copy(kind = if (draft.kind == k) "" else k) }, enabled = (kindCounts[k] ?: 0) > 0)
                }
            }
            // "Added" is offered when it tells titles apart: four rows that all say the whole library are noise.
            val addedCounts = ADDED_DAYS.map { d -> remember(titles, draft, d, state.overlays) { applyFilters(titles, draft.copy(added = d), progress, now, genresOf).size } }
            val everything = remember(titles, draft, state.overlays) { applyFilters(titles, draft.copy(added = 0), progress, now, genresOf).size }
            if (draft.added > 0 || addedCounts.any { it != everything }) {
                SheetHeading(stringResource(R.string.added))
                OptionRow(stringResource(R.string.filter_any), everything, draft.added == 0, { draft = draft.copy(added = 0) })
                ADDED_DAYS.forEachIndexed { i, d ->
                    val n = addedCounts[i]
                    OptionRow(addedLabel(d), n, draft.added == d, { draft = draft.copy(added = if (draft.added == d) 0 else d) }, enabled = n > 0)
                }
            }
            SheetHeading(stringResource(R.string.view))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                ChoiceTile(stringResource(R.string.tiles_large), !draftDense, { draftDense = false }, Modifier.weight(1f))
                ChoiceTile(stringResource(R.string.tiles_small), draftDense, { draftDense = true }, Modifier.weight(1f))
            }
        }
    }
}

/**
 * Six equal columns. `GridCells.Fixed` hands the leftover pixels to the
 * first columns, so the last of three cards came out narrower: its poster
 * ended higher and its caption sat off the row's line. Here the few pixels
 * left over stay at the end, where no one sees them.
 */
private object EvenColumns : GridCells {
    override fun androidx.compose.ui.unit.Density.calculateCrossAxisCellSizes(availableSize: Int, spacing: Int): List<Int> {
        val cell = ((availableSize - spacing * 5) / 6).coerceAtLeast(0)
        return List(6) { cell }
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
 * A title: the hero, the main action on arrival and what else can be done
 * with it (start over, mark watched, download), then its episodes or
 * volumes as rows that say what each is, how long, and where you are.
 */
@OptIn(ExperimentalLayoutApi::class)
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
        if (!title.reading) vm.loadDurations(title.items.map { it.id })
    }
    val next = nextUp(title, state.progress)
    val watched = title.items.count { state.progress[it.id]?.completed == true }
    val series = title.items.size > 1
    val allDone = watched == title.items.size
    // A book opened at its first page has not been started: page 1 is where everyone begins.
    // A book opened and left on its first page is a book being read: every surface says so alike.
    val resume = state.progress[next.id]?.takeIf { !it.completed && it.positionSec > 0.0 }
    val total = title.items.sumOf { state.durations[it.id] ?: 0.0 }
    val more = remember(state.titles, state.overlays, title.key) {
        val mine = state.genresOf(title).map { it.lowercase() }.toSet()
        val others = state.titles.filter { it.key != title.key && it.reading == title.reading }
        val alike = others.map { t -> t to state.genresOf(t).count { it.lowercase() in mine } }.filter { it.second > 0 }
            .sortedWith(compareByDescending<Pair<Title, Int>> { it.second }.thenByDescending { it.first.libraryId == title.libraryId }.thenBy { it.first.title })
            .map { it.first }
        (alike.ifEmpty { others.filter { it.libraryId == title.libraryId } }).take(12)
    }
    // The title's ratings (the social module's, shared with the web); absent on a server without them.
    var rating by remember(title.key) { mutableStateOf<io.github.enrell.lain.core.RatingInfo?>(null) }
    var rateSheet by remember { mutableStateOf(false) }
    // Start over is one tap, and one tap takes it back: the position it cleared, offered back for a while.
    var undone by remember(title.key) { mutableStateOf<io.github.enrell.lain.core.ProgressInfo?>(null) }
    LaunchedEffect(undone) { if (undone != null) { kotlinx.coroutines.delay(8000); undone = null } }
    LaunchedEffect(title.key) { rating = vm.rating(title) }
    var menu by remember { mutableStateOf<Item?>(null) }
    var confirmAll by remember { mutableStateOf(false) }
    var confirmMark by remember { mutableStateOf(false) }
    val ctx = LocalContext.current
    // The copies on this device, read while the screen is up (faster while one is arriving).
    val graph = LocalContext.current.graph
    val scope = rememberCoroutineScope()
    val offline = remember { io.github.enrell.lain.ui.DownloadsModel(graph.core, scope) }
    val copies by offline.state.collectAsState()
    val live by graph.downloads.state.collectAsState()
    LaunchedEffect(live.running) {
        while (true) {
            offline.refresh()
            kotlinx.coroutines.delay(if (live.running) 1_000 else 4_000)
        }
    }
    val copyOf = remember(copies.copies) { copies.copies.associateBy { it.itemId } }
    val ask: (Item) -> Unit = { item -> onDownload(item); offline.refresh() }
    val listState = rememberPageListMemory("title-$key", Screen.TitleDetail(key), vm)
    val pastHero by remember { derivedStateOf { listState.firstVisibleItemIndex > 0 } }
    Box(Modifier.fillMaxSize()) {
    LazyColumn(state = listState, contentPadding = PaddingValues(bottom = 56.dp)) {
        item {
            HeroFrame(title.art(), 320.dp, back = { vm.back() }, lead = 0.2f) {
                // The cover itself, whole and sharp, over its own colours: a title page shows what the title looks like.
                val wideArt = title.wide()
                Box(
                    Modifier.padding(bottom = 16.dp).width(if (wideArt) 196.dp else 112.dp).aspectRatio(if (wideArt) Shapes.LANDSCAPE else Shapes.PORTRAIT)
                        .clip(l.cardShape).border(1.dp, Color.White.copy(alpha = 0.12f), l.cardShape),
                ) { Art(title.art(), Modifier.fillMaxSize(), label = title.title) }
                Text(
                    title.title.uppercase(),
                    color = l.text, fontSize = 30.sp, lineHeight = 31.sp, fontWeight = FontWeight.Black, letterSpacing = (-1.3).sp,
                    maxLines = 3, overflow = TextOverflow.Ellipsis,
                )
                // One line of what it is: year, size or length, then genres.
                val facts = listOfNotNull(
                    title.year.takeIf { it > 0 }?.toString(),
                    if (series) titleSubtitle(title) else null,
                    if (!series && total > 0) length(total) else null,
                ) + state.genresOf(title).take(3)
                if (facts.isNotEmpty()) {
                    Text(facts.joinToString("  ·  "), Modifier.padding(top = 8.dp), color = l.text.copy(alpha = 0.8f), fontSize = 13.sp, lineHeight = 19.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        item {
            Column(Modifier.padding(horizontal = GUTTER)) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                WButton(
                    when {
                        // A series names the volume here and the page under the actions; one book names the page.
                        title.reading && resume != null && series -> stringResource(R.string.action_continue)
                        title.reading && resume != null -> stringResource(R.string.continue_page_short, resume.positionSec.toInt().coerceAtLeast(1))
                        title.reading && allDone -> stringResource(R.string.read_again)
                        title.reading -> stringResource(R.string.action_read)
                        resume != null && series -> stringResource(R.string.action_resume_short)
                        resume != null -> stringResource(R.string.resume_left, length((resume.durationSec - resume.positionSec).coerceAtLeast(0.0)))
                        allDone -> stringResource(R.string.play_again)
                        else -> stringResource(R.string.action_play)
                    } + if (series && (next.episode > 0 || next.season > 0)) " · " + rowLabelShort(next, title.reading) else "",
                    { vm.open(if (allDone) title.items.first() else next) },
                    Modifier.weight(1f),
                    icon = if (title.reading) Ic.Book else Ic.Play,
                    large = true,
                )
                // Keeping it: the account's list, the same one the web client shows.
                state.favorites?.let { favs ->
                    val listed = remember(favs, title.key) { favoriteTitles(listOf(title), favs).isNotEmpty() }
                    val added = stringResource(R.string.fav_added)
                    val removed = stringResource(R.string.fav_removed)
                    Box(
                        Modifier.size(48.dp).clip(l.buttonShape).background(if (listed) l.accent.copy(alpha = 0.2f) else l.surface.copy(alpha = 0.55f))
                            .background(if (listed) l.accent.copy(alpha = 0.16f) else l.tonal).border(1.dp, if (listed) l.accent else Color.Transparent, l.buttonShape)
                            .clickable(onClickLabel = stringResource(if (listed) R.string.fav_remove else R.string.fav_add)) {
                                scope.launch {
                                    if (vm.setFavorite(title, !listed)) android.widget.Toast.makeText(ctx, if (listed) removed else added, android.widget.Toast.LENGTH_SHORT).show()
                                }
                            },
                        contentAlignment = Alignment.Center,
                    ) { Icon(if (listed) Ic.HeartFilled else Ic.Heart, stringResource(if (listed) R.string.fav_remove else R.string.fav_add), tint = if (listed) l.accent else l.text, modifier = Modifier.size(20.dp)) }
                }
                }
                // A film or book under way: how far it is, under the button that resumes it
                // (as the web does). Below the score it sat between unrelated rows and read as a divider.
                // A series shows the part under way the same way: the button names it, the bar says how far.
                resume?.takeIf { it.durationSec > 0 }?.let { p ->
                    Box(Modifier.padding(top = 10.dp).fillMaxWidth().height(3.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.14f))) {
                        Box(Modifier.fillMaxWidth((p.positionSec / p.durationSec).toFloat().coerceIn(0.02f, 1f)).height(3.dp).background(l.accent))
                    }
                }
                // Right under the bar it reads: what is left of the one in progress, and how much of
                // the whole is done. Below the row of actions it was cut off from the bar it explains.
                val under = listOfNotNull(
                    resume?.takeIf { series && !title.reading }?.let { rowLabelShort(next, false) + ": " + stringResource(R.string.time_left, length((it.durationSec - it.positionSec).coerceAtLeast(0.0))) },
                    resume?.takeIf { series && title.reading && it.durationSec > 0 }?.let { rowLabelShort(next, true) + ": " + stringResource(R.string.progress_page, it.positionSec.toInt().coerceAtLeast(1), it.durationSec.toInt()) },
                    if (series && watched > 0) stringResource(if (title.reading) R.string.read_of else R.string.watched_of, watched, title.items.size) else null,
                )
                if (under.isNotEmpty()) Text(under.joinToString("  ·  "), Modifier.padding(top = 8.dp), color = l.muted, fontSize = 13.sp)
                // What else can be done with the title, at one weight below the main action.
                // One row of equal columns, icon over word: sized by their words, the three changed
                // shape from title to title. A word too long for its column takes a second line, and
                // the row grows with it so the columns stay one height.
                Row(Modifier.padding(top = 14.dp).fillMaxWidth().height(androidx.compose.foundation.layout.IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    // On a series it restarts one part, beside actions on the whole title: it names that part.
                    if (resume != null) QuietAction(
                        this, Ic.RotateCcw,
                        if (series && (next.episode > 0 || next.season > 0)) stringResource(R.string.restart_part, rowLabelShort(next, title.reading)) else stringResource(R.string.start_over),
                    ) { undone = resume; vm.markWatched(listOf(next.id), false) }
                    QuietAction(this, 
                        if (allDone) Ic.EyeOff else Ic.CheckCircle,
                        stringResource(
                            when {
                                allDone && series && title.reading -> R.string.mark_all_unread
                                allDone && series -> R.string.mark_all_unwatched
                                allDone -> R.string.mark_unwatched
                                series && title.reading -> R.string.mark_all_read
                                series -> R.string.mark_all_watched
                                title.reading -> R.string.mark_read
                                else -> R.string.mark_watched
                            },
                        ),
                    ) { if (series) confirmMark = true else vm.markWatched(title.items.map { it.id }, !allDone) }
                    val have = title.items.count { copyOf[it.id]?.state == io.github.enrell.lain.core.CopyState.DONE }
                    if (have == title.items.size) {
                        QuietAction(this, Ic.Check, stringResource(R.string.dl_downloaded)) { vm.go(Screen.Downloads) }
                    } else {
                        // With some already here, the button says it fetches the others.
                        QuietAction(this, Ic.Download, stringResource(if (series && have > 0) R.string.download_rest else if (series) R.string.download_all else R.string.action_download)) {
                            if (series) confirmAll = true else title.items.filter { copyOf[it.id] == null }.forEach(ask)
                        }
                    }
                }
                // Your score and everyone's: one tap to give or change it.
                rating?.let { r ->
                    val mine = r.mine.toInt()
                    Row(Modifier.padding(top = 14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Row(
                            Modifier.height(36.dp).clip(CircleShape).background(if (mine > 0) l.accent.copy(alpha = 0.16f) else l.tonal)
                                .clickable(onClickLabel = stringResource(R.string.rate_title)) { rateSheet = true }.padding(horizontal = 14.dp),
                            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(7.dp),
                        ) {
                            Icon(if (mine > 0) Ic.StarFilled else Ic.Star, null, tint = if (mine > 0) l.accent else l.text, modifier = Modifier.size(15.dp))
                            Text(if (mine > 0) stringResource(R.string.rate_yours, mine) else stringResource(R.string.rate_action), color = l.text, fontSize = 13.sp, fontWeight = FontWeight.Medium)
                        }
                        // An average of one, when the one is yours, would say your score twice.
                        if (r.scored.toInt() > 1 || (r.scored.toInt() == 1 && mine == 0)) {
                            Text(
                                pluralStringResource(R.plurals.rate_average, r.scored.toInt(), java.lang.String.format(java.util.Locale.US, "%.1f", r.average), r.scored.toInt()),
                                color = l.muted, fontSize = 13.sp, maxLines = 1,
                            )
                        }
                    }
                }
            }
        }
        state.synopsisOf(title).takeIf { it.isNotEmpty() }?.let { text ->
            item {
                var open by remember { mutableStateOf(false) }
                Text(
                    text,
                    Modifier.padding(horizontal = GUTTER).padding(top = 16.dp).clickable { open = !open },
                    color = l.text.copy(alpha = 0.72f), fontSize = 14.sp, lineHeight = 22.sp, maxLines = if (open) 40 else 3, overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (series) {
            item {
                Text(
                    stringResource(if (title.reading) R.string.volumes else R.string.episodes),
                    Modifier.padding(start = GUTTER, top = 24.dp, bottom = 10.dp),
                    color = l.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.3).sp,
                )
            }
            items(title.items, key = { it.id }) { item ->
                val c = copyOf[item.id]
                val pct = c?.let { live.bytes[it.itemId] ?: (it.bytes to it.size) }?.let { (d, t) -> if (t > 0) (d * 100 / t).toInt().coerceIn(0, 100) else 0 }
                EpisodeRow(item, title, state, { vm.open(item) }, { menu = item }, c?.state, pct ?: 0)
            }
        }
        item {
            Column(Modifier.padding(horizontal = GUTTER).padding(top = 26.dp).fillMaxWidth()) {
                Mono(stringResource(R.string.details))
                Gap(8.dp)
                state.libraries.firstOrNull { it.id == title.libraryId }?.let { Fact(stringResource(R.string.nav_library), it.name) }
                if (series && title.year > 0) Fact(stringResource(R.string.fact_year), title.year.toString())
                if (series) Fact(stringResource(if (title.reading) R.string.volumes else R.string.episodes), title.items.size.toString())
                if (series && total > 0) Fact(stringResource(R.string.fact_length), length(total))
                // What a download of it takes on this device.
                Fact(stringResource(R.string.fact_download_size), bytes(title.items.sumOf { it.size }))
                Box(Modifier.fillMaxWidth().height(1.dp).background(l.line.copy(alpha = 0.6f)))
            }
        }
        // Where to go next: titles that share a genre with this one (its own
        // library first), or its library's others when nothing is known of it.
        if (more.isNotEmpty()) {
            item {
                Column(Modifier.padding(top = 26.dp)) {
                    RailHeader(stringResource(R.string.more_like_this), null, null)
                    LazyRow(Modifier.padding(top = 8.dp), contentPadding = PaddingValues(horizontal = GUTTER), horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                        items(more, key = { it.key }) { RailCard(it, vm, seen = titleFraction(it, allProgressOf(state))) }
                    }
                }
            }
        }
    }
    if (rateSheet) {
        val mine = rating?.mine?.toInt() ?: 0
        val saved = stringResource(R.string.rate_saved)
        val removed = stringResource(R.string.rate_removed)
        val failed = stringResource(R.string.rate_failed)
        val give = { score: Int ->
            rateSheet = false
            scope.launch {
                val got = vm.rate(title, score)
                if (got != null) rating = got
                android.widget.Toast.makeText(ctx, if (got == null) failed else if (score == 0) removed else saved, android.widget.Toast.LENGTH_SHORT).show()
            }
            Unit
        }
        Sheet(stringResource(R.string.rate_title), { rateSheet = false }) {
            Text(stringResource(R.string.rate_body, title.title), color = l.muted, fontSize = 14.sp, lineHeight = 20.sp)
            Gap(14.dp)
            for (row in (1..10).chunked(5)) {
                Row(Modifier.fillMaxWidth().padding(vertical = 3.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    for (n in row) ChoiceTile(n.toString(), mine == n, { give(n) }, Modifier.weight(1f), ring = false)
                }
            }
            if (mine > 0) {
                Gap(12.dp)
                WButton(stringResource(R.string.rate_remove), { give(0) }, Modifier.fillMaxWidth(), kind = Btn.Secondary)
            }
        }
    }
    // Scrolled past the hero, the page keeps a top of its own: the way back and
    // the name, on solid ground, so nothing runs up into the status bar.
    androidx.compose.animation.AnimatedVisibility(pastHero, enter = androidx.compose.animation.fadeIn(), exit = androidx.compose.animation.fadeOut()) {
        Column(Modifier.fillMaxWidth().background(l.bg)) {
            Row(Modifier.fillMaxWidth().height(56.dp).padding(start = 6.dp, end = GUTTER), verticalAlignment = Alignment.CenterVertically) {
                IconBtn(Ic.ArrowLeft, "Back", { vm.back() }, tint = l.text)
                Text(title.title, Modifier.padding(start = 4.dp), color = l.text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Box(Modifier.fillMaxWidth().height(1.dp).background(l.line.copy(alpha = 0.7f)))
        }
    }
    undone?.let { was ->
        Row(
            Modifier.align(Alignment.BottomCenter).padding(horizontal = GUTTER, vertical = 16.dp).fillMaxWidth().clip(l.buttonShape).background(l.raised).border(1.dp, l.line, l.buttonShape).padding(start = 14.dp, end = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(stringResource(R.string.started_over), Modifier.weight(1f).padding(vertical = 12.dp), color = l.text, fontSize = 14.sp)
            Text(
                stringResource(R.string.undo),
                Modifier.clip(l.buttonShape).clickable { undone = null; vm.restoreProgress(was) }.padding(horizontal = 12.dp, vertical = 12.dp),
                color = l.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold,
            )
        }
    }
    }
    if (confirmMark) {
        // A whole title's marks change at once: asked first, since there is no undo.
        val n = title.items.size
        Sheet(stringResource(if (allDone) R.string.mark_all_unwatched_q else if (title.reading) R.string.mark_all_read_q else R.string.mark_all_watched_q), { confirmMark = false }, footer = {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                WButton(stringResource(R.string.player_cancel), { confirmMark = false }, Modifier.weight(1f), kind = Btn.Secondary)
                WButton(stringResource(R.string.confirm_mark), { confirmMark = false; vm.markWatched(title.items.map { it.id }, !allDone) }, Modifier.weight(1f))
            }
        }) {
            Text(
                (if (title.reading) pluralStringResource(R.plurals.count_volumes, n, n) else pluralStringResource(R.plurals.count_episodes, n, n)) + "  ·  " + title.title,
                color = l.text, fontSize = 16.sp, fontWeight = FontWeight.Medium,
            )
            Text(stringResource(R.string.mark_all_body), Modifier.padding(top = 6.dp), color = l.muted, fontSize = 14.sp, lineHeight = 20.sp)
        }
    }
    if (confirmAll) {
        // A whole title is a lot of room: say how much before taking it.
        val missing = title.items.filter { copyOf[it.id] == null }
        Sheet(stringResource(R.string.download_all_q), { confirmAll = false }, footer = {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                WButton(stringResource(R.string.player_cancel), { confirmAll = false }, Modifier.weight(1f), kind = Btn.Secondary)
                WButton(stringResource(R.string.action_download), { confirmAll = false; missing.forEach(ask) }, Modifier.weight(1f), icon = Ic.Download)
            }
        }) {
            Text(
                (if (title.reading) pluralStringResource(R.plurals.count_volumes, missing.size, missing.size) else pluralStringResource(R.plurals.count_episodes, missing.size, missing.size)) + "  ·  " + bytes(missing.sumOf { it.size }),
                color = l.text, fontSize = 16.sp, fontWeight = FontWeight.Medium,
            )
            Text(stringResource(R.string.download_all_body), Modifier.padding(top = 6.dp), color = l.muted, fontSize = 14.sp, lineHeight = 20.sp)
        }
    }
    menu?.let { item ->
        val p = state.progress[item.id]
        Sheet(rowLabel(item, title.reading), { menu = null }) {
            SheetAction(Ic.Play, stringResource(if (title.reading) R.string.action_read else R.string.action_play)) { menu = null; vm.open(item) }
            if (p != null && !p.completed && p.positionSec > 0) SheetAction(Ic.RotateCcw, stringResource(R.string.start_over)) { menu = null; undone = p; vm.markWatched(listOf(item.id), false) }
            SheetAction(if (p?.completed == true) Ic.EyeOff else Ic.CheckCircle, stringResource(if (p?.completed == true) R.string.mark_unwatched else if (title.reading) R.string.mark_read else R.string.mark_watched)) {
                menu = null
                vm.markWatched(listOf(item.id), p?.completed != true)
            }
            if (copyOf[item.id] == null) {
                SheetAction(Ic.Download, stringResource(R.string.action_download)) { menu = null; ask(item) }
            } else {
                SheetAction(Ic.Trash, stringResource(R.string.remove_download)) { menu = null; offline.remove(item.id) }
            }
        }
    }
}

/** `Episode 3`, `Volume 1`, or the file's own name when it carries no number. */
@Composable
private fun rowLabel(item: Item, reading: Boolean): String = when {
    reading && item.season > 0 -> stringResource(R.string.volume_long, item.season.toInt()) + if (item.episode > 0) " · #${item.episode}" else ""
    reading && item.episode > 0 -> stringResource(R.string.volume_long, item.episode.toInt())
    item.episode > 0 && item.season > 0 -> stringResource(R.string.season_episode, item.season.toInt(), item.episode.toInt())
    item.episode > 0 -> stringResource(R.string.episode_n, item.episode.toInt())
    else -> item.fileName.substringBeforeLast('.')
}

@Composable
private fun rowLabelShort(item: Item, reading: Boolean): String = when {
    reading && item.season > 0 -> "Vol ${item.season}"
    reading -> episodeLabel(item.season, item.episode, true)
    item.episode > 0 -> stringResource(R.string.episode_n, item.episode.toInt())
    else -> episodeLabel(item.season, item.episode, false)
}

/** A secondary action on a title: one of a row of equal columns, its icon over its word. */
@Composable
private fun QuietAction(row: androidx.compose.foundation.layout.RowScope, icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, onClick: () -> Unit) {
    val l = LocalLain.current
    with(row) {
        Column(
            Modifier.weight(1f).fillMaxHeight().heightIn(min = 60.dp).clip(l.buttonShape).background(l.tonal)
                .clickable(onClickLabel = label, onClick = onClick).padding(horizontal = 4.dp, vertical = 8.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(5.dp, Alignment.CenterVertically),
        ) {
            Icon(icon, null, tint = l.text, modifier = Modifier.size(18.dp))
            Text(
                label, color = l.text.copy(alpha = 0.85f), fontSize = 12.sp, lineHeight = 15.sp, fontWeight = FontWeight.Medium,
                textAlign = androidx.compose.ui.text.style.TextAlign.Center, maxLines = 2, overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun SheetAction(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, onClick: () -> Unit) {
    val l = LocalLain.current
    Row(
        Modifier.fillMaxWidth().heightIn(min = 52.dp).clip(l.buttonShape).clickable(onClick = onClick).padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Icon(icon, null, tint = l.muted, modifier = Modifier.size(20.dp))
        Text(label, color = l.text, fontSize = 15.sp, fontWeight = FontWeight.Medium)
    }
}

@Composable
private fun Fact(label: String, value: String) {
    val l = LocalLain.current
    Column {
        Box(Modifier.fillMaxWidth().height(1.dp).background(l.line.copy(alpha = 0.6f)))
        Row(Modifier.fillMaxWidth().padding(vertical = 11.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(label, Modifier.weight(1f), color = l.muted, fontSize = 14.sp)
            Text(value, color = l.text, fontSize = 14.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/**
 * An episode or volume as a row: its picture with the progress on it, what
 * it is, and one line that is always the same kind of fact (where you are
 * in it, or how long it is). A tap plays; the dots open what else it can do.
 */
@Composable
private fun EpisodeRow(item: Item, title: Title, state: AppState, onOpen: () -> Unit, onMenu: () -> Unit, copy: io.github.enrell.lain.core.CopyState? = null, copyPercent: Int = 0) {
    val l = LocalLain.current
    val p = state.progress[item.id]
    val dur = state.durations[item.id] ?: p?.durationSec?.takeIf { it > 0 && !title.reading }
    Row(
        Modifier.padding(horizontal = GUTTER, vertical = 4.dp).fillMaxWidth().clip(l.cardShape)
            .background(l.tonalLow).clickable(onClickLabel = stringResource(if (title.reading) R.string.action_read else R.string.action_play), onClick = onOpen),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // A volume shows its whole cover, at a cover's shape; an episode its still.
        Box(Modifier.width(if (title.reading) 84.dp else 132.dp).aspectRatio(if (title.reading) 2f / 3f else 16f / 9f)) {
            Art(item.id, Modifier.fillMaxSize())
            if (p?.completed == true) Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.45f)))
            val f = if (p?.completed == true) 0f else fraction(p)
            if (f > 0f) {
                Box(Modifier.align(Alignment.BottomStart).padding(horizontal = 7.dp, vertical = 7.dp).fillMaxWidth().height(5.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.65f))) {
                    Box(Modifier.fillMaxWidth(f).height(5.dp).clip(CircleShape).background(l.accent))
                }
            }
            // No play disc on every still: the whole row plays, and a glyph repeated down
            // the list only buried the marks that differ (watched, under way).
            if (p?.completed == true) {
                Box(Modifier.align(Alignment.TopEnd).padding(6.dp).size(20.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.6f)), contentAlignment = Alignment.Center) {
                    Icon(Ic.Check, stringResource(R.string.progress_done), tint = Color.White, modifier = Modifier.size(12.dp))
                }
            }
        }
        Column(Modifier.weight(1f).padding(start = 14.dp)) {
            Text(rowLabel(item, title.reading), color = l.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                when {
                    p?.completed == true && dur != null -> length(dur) + " · " + stringResource(if (title.reading) R.string.progress_done else R.string.watched)
                    p?.completed == true -> stringResource(if (title.reading) R.string.progress_done else R.string.watched)
                    // Page 1 is where everyone begins: a volume opened and closed has not been started.
                    p != null && p.positionSec >= 1 && title.reading -> stringResource(R.string.progress_page, p.positionSec.toInt(), p.durationSec.toInt())
                    p != null && title.reading && p.completed != true -> stringResource(R.string.watch_unwatched)
                    p != null && p.positionSec > 0 && p.durationSec > 0 -> stringResource(R.string.time_left, length((p.durationSec - p.positionSec).coerceAtLeast(0.0)))
                    dur != null -> length(dur)
                    else -> stringResource(R.string.watch_unwatched)
                },
                Modifier.padding(top = 4.dp),
                color = when {
                    p?.completed == true -> l.muted.copy(alpha = 0.8f)
                    p != null && p.positionSec > (if (title.reading) 1.0 else 0.0) -> l.accent
                    else -> l.muted
                },
                fontSize = 13.sp, maxLines = 1,
            )
            // Where its copy on this device is, when it has or is getting one.
            if (copy != null) {
                Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                    val done = copy == io.github.enrell.lain.core.CopyState.DONE
                    val failed = copy == io.github.enrell.lain.core.CopyState.FAILED
                    Icon(if (done) Ic.Check else Ic.Download, null, tint = if (done) l.ok else if (failed) l.danger else l.accent, modifier = Modifier.size(12.dp))
                    Text(
                        when (copy) {
                            io.github.enrell.lain.core.CopyState.DONE -> stringResource(R.string.dl_downloaded)
                            io.github.enrell.lain.core.CopyState.FAILED -> stringResource(R.string.dl_failed)
                            io.github.enrell.lain.core.CopyState.PAUSED -> stringResource(R.string.dl_queued)
                            else -> if (copyPercent > 0) stringResource(R.string.dl_downloading, copyPercent) else stringResource(R.string.dl_queued)
                        },
                        color = if (done) l.ok else if (failed) l.danger else l.muted, fontSize = 12.sp, maxLines = 1,
                    )
                }
            }
        }
        IconBtn(Ic.Dots, stringResource(R.string.more_actions), onMenu, tint = l.text.copy(alpha = 0.85f))
    }
}

/** Search: one field with the focus; recent searches and the libraries to start from; titles as cards. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun SearchScreen(state: AppState, vm: AppViewModel) {
    val l = LocalLain.current
    val prefs = LocalContext.current.graph.prefs
    var recents by remember { mutableStateOf(prefs.recentSearches) }
    // A search that found something and stayed is worth remembering.
    LaunchedEffect(state.query, state.results.isNotEmpty()) {
        val q = state.query.trim()
        if (q.length >= 2 && state.results.isNotEmpty()) {
            kotlinx.coroutines.delay(1500)
            recents = (listOf(q) + recents.filterNot { it.equals(q, ignoreCase = true) }).take(8)
            prefs.recentSearches = recents
        }
    }
    val focus = remember { androidx.compose.ui.focus.FocusRequester() }
    LaunchedEffect(Unit) { if (state.query.isEmpty()) runCatching { focus.requestFocus() } }
    Column(Modifier.fillMaxSize()) {
        SearchField(state.query, { vm.search(it) }, stringResource(R.string.search_hint), Modifier.padding(horizontal = GUTTER).padding(top = 14.dp).fillMaxWidth(), focus = focus)
        when {
            state.query.isBlank() -> Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = GUTTER).padding(top = 22.dp, bottom = 32.dp)) {
                if (recents.isNotEmpty()) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Mono(stringResource(R.string.recent_searches), Modifier.weight(1f))
                        // Its word ends on the content edge, its touch area runs into the gutter.
                        Text(stringResource(R.string.clear), Modifier.offset(x = 8.dp).clip(CircleShape).clickable { recents = emptyList(); prefs.recentSearches = emptyList() }.padding(horizontal = 8.dp, vertical = 6.dp), color = l.muted, fontSize = 12.sp)
                    }
                    FlowRow(Modifier.padding(top = 8.dp, bottom = 26.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        for (q in recents) {
                            Row(
                                Modifier.height(40.dp).clip(CircleShape).background(l.tonal).clickable { vm.search(q) }.padding(horizontal = 14.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(7.dp),
                            ) {
                                Text(q, color = l.text, fontSize = 14.sp, maxLines = 1)
                            }
                        }
                    }
                }
                val libraries = remember(state.titles, state.libraries) { state.libraries.filter { lib -> state.titles.any { it.libraryId == lib.id } } }
                if (libraries.isNotEmpty()) {
                    Mono(stringResource(R.string.browse))
                    Gap(6.dp)
                    for (lib in libraries) {
                        val n = remember(state.titles, lib.id) { state.titles.count { it.libraryId == lib.id } }
                        Row(
                            // On the gutter, as the same rows under a query: the list must not shift as you type.
                            Modifier.fillMaxWidth().heightIn(min = 52.dp).clip(l.buttonShape).clickable { openLibrary(vm, lib.id) },
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(lib.name, Modifier.weight(1f), color = l.text, fontSize = 15.sp, fontWeight = FontWeight.Medium)
                            Text(pluralStringResource(R.plurals.count_titles, n, n), color = l.muted, fontSize = 13.sp)
                            Icon(Ic.ChevronRight, null, tint = l.muted, modifier = Modifier.padding(start = 8.dp).size(16.dp))
                        }
                        Box(Modifier.fillMaxWidth().height(1.dp).background(l.line.copy(alpha = 0.5f)))
                    }
                }
            }
            state.searching && state.results.isEmpty() -> Mono(stringResource(R.string.working), Modifier.padding(GUTTER))
            state.results.isEmpty() -> Text(stringResource(R.string.search_none), Modifier.padding(GUTTER), color = l.muted, fontSize = 14.sp)
            else -> {
                val n = remember(state.results) { state.results.map { titleKey(it) }.distinct().size }
                Text(pluralStringResource(R.plurals.results_for, n, n, state.query.trim()), Modifier.padding(start = GUTTER, top = 14.dp), color = l.muted, fontSize = 14.sp)
                // A search answers with titles, not files: twelve episodes of one show
                // are one card that opens the show.
                val found = remember(state.results, state.titles) {
                    val byKey = state.titles.associateBy { it.key }
                    state.results.map { titleKey(it) }.distinct().mapNotNull { byKey[it] }
                }
                // Rows, not posters: several answers stay in sight above the keyboard,
                // and the list ends clear of it.
                val resultsState = androidx.compose.foundation.lazy.rememberLazyListState()
                val keyboard = androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
                LaunchedEffect(resultsState.isScrollInProgress) { if (resultsState.isScrollInProgress) keyboard?.hide() }
                val libraryName = remember(state.libraries) { state.libraries.associate { it.id to it.name } }
                androidx.compose.foundation.lazy.LazyColumn(
                    state = resultsState,
                    modifier = Modifier.fillMaxSize().imePadding(),
                    contentPadding = PaddingValues(top = 10.dp, bottom = 32.dp),
                ) {
                    items(found.size, key = { found[it].key }) { i ->
                        val t = found[i]
                        val wideArt = t.wide()
                        Row(
                            Modifier.fillMaxWidth().clickable(onClickLabel = t.title) { keyboard?.hide(); vm.go(Screen.TitleDetail(t.key)) }.padding(horizontal = GUTTER, vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Box(Modifier.width(if (wideArt) 96.dp else 54.dp).aspectRatio(if (wideArt) Shapes.LANDSCAPE else Shapes.PORTRAIT).clip(RoundedCornerShape(6.dp))) {
                                Art(t.art(), Modifier.fillMaxSize(), label = t.title)
                            }
                            Column(Modifier.weight(1f).padding(start = 14.dp)) {
                                Text(t.title, color = l.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis)
                                Text(
                                    listOfNotNull(libraryName[t.libraryId], progressSubtitle(t, allProgressOf(state)) ?: titleSubtitle(t)).filter { it.isNotEmpty() }.joinToString(" · "),
                                    Modifier.padding(top = 3.dp), color = l.muted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                                )
                            }
                            Icon(Ic.ChevronRight, null, tint = l.muted, modifier = Modifier.size(16.dp))
                        }
                    }
                    // A short answer leaves somewhere to go on to: the libraries, as before a query.
                    if (found.size <= 6) {
                        item(key = "@browse") {
                            Column(Modifier.padding(top = 18.dp)) {
                                Mono(stringResource(R.string.search_or_browse), Modifier.padding(horizontal = GUTTER, vertical = 6.dp))
                                // The same rows as before a query: name, count, a divider.
                                for (lib in state.libraries.filter { !it.locked }) {
                                    val n = state.titles.count { it.libraryId == lib.id }
                                    Row(
                                        Modifier.fillMaxWidth().clickable { keyboard?.hide(); openLibrary(vm, lib.id) }.padding(horizontal = GUTTER, vertical = 13.dp),
                                        verticalAlignment = Alignment.CenterVertically,
                                    ) {
                                        Text(lib.name, Modifier.weight(1f), color = l.text, fontSize = 15.sp, fontWeight = FontWeight.Medium)
                                        Text(pluralStringResource(R.plurals.count_titles, n, n), color = l.muted, fontSize = 13.sp)
                                        Icon(Ic.ChevronRight, null, tint = l.muted, modifier = Modifier.padding(start = 8.dp).size(16.dp))
                                    }
                                    Box(Modifier.padding(horizontal = GUTTER).fillMaxWidth().height(1.dp).background(l.line.copy(alpha = 0.5f)))
                                }
                            }
                        }
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
    val ctx = LocalContext.current
    val done = stringResource(R.string.private_unlocked)
    fun submit() {
        if (secret.isEmpty() || busy) return
        busy = true
        scope.launch {
            error = vm.unlock(secret, usePin)
            busy = false
            if (error == null) {
                android.widget.Toast.makeText(ctx, done, android.widget.Toast.LENGTH_SHORT).show()
                onClose()
            } else {
                secret = ""
            }
        }
    }
    Sheet(stringResource(R.string.unlock_title), onClose) {
        Text(stringResource(R.string.unlock_body), color = l.muted, fontSize = 14.sp, lineHeight = 20.sp)
        Gap(16.dp)
        if (usePin) {
            PinField(secret, { secret = it; error = null }, { submit() }, Modifier.fillMaxWidth(), stringResource(R.string.unlock_pin))
        } else {
            Text(stringResource(R.string.login_password), color = l.muted, fontSize = 14.sp, fontWeight = FontWeight.Medium)
            Gap(6.dp)
            SecretField(secret, { secret = it; error = null }, numeric = false, onDone = { submit() }, modifier = Modifier.fillMaxWidth())
        }
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


/**
 * A hero: the art across the whole top, and the words at the bottom on a
 * gradient that has reached the page's ground by the time the first line
 * starts, so lettering printed on a poster never sits under them. The
 * height leaves the next section on screen.
 */
@Composable
private fun HeroFrame(artId: String, minHeight: Dp, back: (() -> Unit)? = null, lead: Float = 0.52f, content: @Composable ColumnScope.() -> Unit) {
    val l = LocalLain.current
    Box(Modifier.fillMaxWidth().heightIn(min = minHeight)) {
        // The art's colours wash the whole block, so the title and facts sit on
        // colour and not on a black void; the picture itself is only the top
        // part and melts into that wash, so its own lettering never reaches the words.
        Ambient(artId, Modifier.matchParentSize(), strength = 0.9f)
        Box(Modifier.matchParentSize().background(Brush.verticalGradient(0f to Color.Transparent, 0.5f to l.bg.copy(alpha = 0.35f), 0.86f to l.bg.copy(alpha = 0.62f), 1f to l.bg)))
        Box(
            // Masked, not covered: the picture fades to nothing over the wash.
            Modifier.fillMaxWidth().height(minHeight * 0.52f)
                .graphicsLayer { compositingStrategy = androidx.compose.ui.graphics.CompositingStrategy.Offscreen }
                .drawWithContent {
                    drawContent()
                    drawRect(Brush.verticalGradient(0f to Color.Black, 0.35f to Color.Black, 1f to Color.Transparent), blendMode = androidx.compose.ui.graphics.BlendMode.DstIn)
                },
        ) {
            Art(artId, Modifier.matchParentSize(), alignment = Alignment.TopCenter)
            Box(Modifier.matchParentSize().background(Brush.verticalGradient(0f to Color.Black.copy(alpha = 0.3f), 0.25f to Color.Transparent)))
        }
        Column(Modifier.align(Alignment.BottomStart).fillMaxWidth().padding(horizontal = GUTTER).padding(top = minHeight * lead, bottom = 18.dp), content = content)
        if (back != null) Box(Modifier.padding(8.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.45f))) { IconBtn(Ic.ArrowLeft, "Back", back, tint = Color.White) }
    }
}

/** Pull down to read the catalog again. */
@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
private fun Refreshable(refreshing: Boolean, onRefresh: () -> Unit, content: @Composable () -> Unit) {
    val l = LocalLain.current
    val pull = androidx.compose.material3.pulltorefresh.rememberPullToRefreshState()
    androidx.compose.material3.pulltorefresh.PullToRefreshBox(
        isRefreshing = refreshing,
        onRefresh = onRefresh,
        state = pull,
        indicator = {
            androidx.compose.material3.pulltorefresh.PullToRefreshDefaults.Indicator(
                state = pull, isRefreshing = refreshing, modifier = Modifier.align(Alignment.TopCenter), containerColor = l.raised, color = l.accent,
            )
        },
    ) { content() }
}

/** Home while the first catalog arrives: the shape of what is coming. */
@Composable
private fun HomeSkeleton() {
    Column(Modifier.fillMaxSize().padding(GUTTER), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Skeleton(Modifier.fillMaxWidth().height(260.dp))
        Skeleton(Modifier.fillMaxWidth(0.7f).height(34.dp))
        Skeleton(Modifier.fillMaxWidth(0.45f).height(14.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Skeleton(Modifier.width(110.dp).height(46.dp))
            Skeleton(Modifier.width(120.dp).height(46.dp))
        }
        Gap(8.dp)
        Skeleton(Modifier.fillMaxWidth(0.4f).height(20.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(14.dp)) { repeat(3) { Skeleton(Modifier.width(RAIL_H * Shapes.PORTRAIT).height(RAIL_H)) } }
    }
}


/**
 * Where lists were, by screen: the stack keeps no screen alive, so a list
 * that is come back to would start at its top. In memory only.
 */
object ListMemory {
    val at = HashMap<String, Pair<Int, Int>>()

    /** Forgets the places under a prefix: a screen opened afresh starts at its top. */
    fun forget(prefix: String) {
        at.keys.removeAll { it.startsWith(prefix) }
    }

    /**
     * Lists asked to go back to their top, by key. An event, not a state: forgetting the
     * place only helps a list composed later, while the bar's tab tapped again is about the
     * list already on screen; one that is not on screen hears nothing and keeps its place.
     */
    val tops = kotlinx.coroutines.flow.MutableSharedFlow<String>(extraBufferCapacity = 4)

    /** Takes the list under [key] to its top, the one on screen now included. */
    fun toTop(key: String) {
        forget(key)
        tops.tryEmit(key)
    }
}

/** Opens a library at its top (only Back returns to where a list was). */
private fun openLibrary(vm: AppViewModel, id: String) {
    ListMemory.forget("grid-$id")
    vm.go(Screen.LibraryDetail(id))
}

@Composable
private fun rememberListMemory(key: String): androidx.compose.foundation.lazy.LazyListState {
    val saved = ListMemory.at[key]
    val state = androidx.compose.foundation.lazy.rememberLazyListState(saved?.first ?: 0, saved?.second ?: 0)
    DisposableEffect(key) { onDispose { ListMemory.at[key] = state.firstVisibleItemIndex to state.firstVisibleItemScrollOffset } }
    LaunchedEffect(key) { ListMemory.tops.collect { if (it == key) state.animateScrollToItem(0) } }
    return state
}

/**
 * The list of a page that keeps its place while the page can be returned to (under the
 * player an episode row opened, or in a destination left for another). Without it a show's
 * page came back at its top after every episode, far from the next one. A page closed with
 * Back forgets its place: opened again, it starts at its top.
 */
@Composable
private fun rememberPageListMemory(key: String, page: Screen, vm: AppViewModel): androidx.compose.foundation.lazy.LazyListState {
    val saved = ListMemory.at[key]
    val state = androidx.compose.foundation.lazy.rememberLazyListState(saved?.first ?: 0, saved?.second ?: 0)
    DisposableEffect(key) {
        onDispose {
            // Read after the navigation that removed the page, so it says where the page went.
            if (vm.state.value.keeps(page)) ListMemory.at[key] = state.firstVisibleItemIndex to state.firstVisibleItemScrollOffset
            else ListMemory.at.remove(key)
        }
    }
    return state
}

@Composable
private fun rememberGridMemory(key: String): androidx.compose.foundation.lazy.grid.LazyGridState {
    val saved = ListMemory.at[key]
    val state = rememberLazyGridState(saved?.first ?: 0, saved?.second ?: 0)
    DisposableEffect(key) { onDispose { ListMemory.at[key] = state.firstVisibleItemIndex to state.firstVisibleItemScrollOffset } }
    return state
}


/** Everything known of the user's progress (what is in progress and what a title page loaded), kept while it does not change. */
@Composable
private fun allProgressOf(state: AppState): Map<String, io.github.enrell.lain.core.ProgressInfo> =
    remember(state.continues, state.progress) { state.allProgress() }
