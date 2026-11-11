package io.github.enrell.lain.domain

import io.github.enrell.lain.core.Item
import io.github.enrell.lain.core.LibraryInfo
import io.github.enrell.lain.core.ProgressInfo

/** Item kinds that open in the reader instead of the player. */
val READING_KINDS = setOf("comic", "manga", "book")

fun Item.reading(): Boolean = kind in READING_KINDS

/**
 * One title: the items of a library that share a title, in episode
 * order. The desktop groups the catalog the same way (D-181).
 */
data class Title(
    val key: String,
    val title: String,
    val kind: String,
    val libraryId: String,
    val items: List<Item>,
    val latest: Long,
    val year: Long,
) {
    val reading: Boolean get() = kind in READING_KINDS
}

fun titleKey(item: Item): String = item.libraryId + "\u0000" + item.title.lowercase()

/** Groups the catalog into titles, sorted by name; missing files are left out (D-068). */
fun groupTitles(items: List<Item>): List<Title> {
    val groups = LinkedHashMap<String, MutableList<Item>>()
    for (item in items) {
        if (item.missing) continue
        groups.getOrPut(titleKey(item)) { mutableListOf() }.add(item)
    }
    return groups.map { (key, list) ->
        val sorted = list.sortedWith(compareBy<Item>({ it.season }, { it.episode }, { it.fileName }))
        val first = sorted.first()
        // Spellings differ only in case within a group; show the one the
        // most recently updated item carries (the latest metadata).
        val newest = sorted.maxBy { it.updatedAt }
        Title(
            key = key,
            title = newest.title,
            kind = first.kind,
            libraryId = first.libraryId,
            items = sorted,
            latest = sorted.maxOf { it.updatedAt },
            year = sorted.firstOrNull { it.year != 0L }?.year ?: 0L,
        )
    }.sortedBy { it.title.lowercase() }
}

/** What a rail is called; the screen translates it. */
sealed interface RailLabel {
    data object Continue : RailLabel
    data object Recent : RailLabel

    /** What is being read, apart from what is being watched. */
    data object ContinueReading : RailLabel
    /** The titles kept as favourites. */
    data object MyList : RailLabel
    data class Library(val name: String) : RailLabel
}

sealed interface Card {
    data class Resume(val item: Item, val progress: ProgressInfo) : Card
    data class Show(val title: Title) : Card
}

data class Rail(val label: RailLabel, val cards: List<Card>)

/** Home: Continue, Recently added (30), then one rail per library. Empty rails are dropped. */
fun buildRails(
    titles: List<Title>,
    continues: List<ProgressInfo>,
    items: Map<String, Item>,
    libraries: List<LibraryInfo>,
    favorites: List<Title> = emptyList(),
): List<Rail> {
    val rails = mutableListOf<Rail>()
    val resume = continues.filter { !it.completed }.mapNotNull { p -> items[p.itemId]?.let { Card.Resume(it, p) } }
    rails += Rail(RailLabel.Continue, resume.filter { !it.item.reading() })
    rails += Rail(RailLabel.ContinueReading, resume.filter { it.item.reading() })
    rails += Rail(RailLabel.MyList, favorites.map { Card.Show(it) })
    rails += Rail(RailLabel.Recent, titles.sortedByDescending { it.latest }.take(30).map { Card.Show(it) })
    val byLibrary = titles.groupBy { it.libraryId }
    val named = libraries.associateBy { it.id }
    for ((id, list) in byLibrary) {
        val name = named[id]?.name ?: continue
        rails += Rail(RailLabel.Library(name), list.map { Card.Show(it) })
    }
    return rails.filter { it.cards.isNotEmpty() }
}

/** The item a title's main action opens: the first one not finished, else the first. */
fun nextUp(title: Title, progress: Map<String, ProgressInfo>): Item {
    // The one in progress; else the one after the last finished (watching
    // episode 2 leads to 3, whatever happened to 1); else the first.
    title.items.firstOrNull { progress[it.id]?.let { p -> !p.completed && p.positionSec > 0 } == true }?.let { return it }
    val last = title.items.indexOfLast { progress[it.id]?.completed == true }
    return title.items.drop(last + 1).firstOrNull { progress[it.id]?.completed != true }
        ?: title.items.firstOrNull { progress[it.id]?.completed != true }
        ?: title.items.first()
}

/** How far along an item is, 0..1. */
fun fraction(p: ProgressInfo?): Float {
    if (p == null) return 0f
    if (p.completed) return 1f
    if (p.durationSec <= 0.0) return 0f
    return (p.positionSec / p.durationSec).toFloat().coerceIn(0f, 1f)
}

/**
 * The titles a list of favourites names. A favourite is a work (kind and
 * title): it points at one catalog item, and any item of the title, or the
 * same name, makes the match.
 */
fun favoriteTitles(titles: List<Title>, favorites: List<io.github.enrell.lain.core.FavoriteInfo>): List<Title> {
    if (favorites.isEmpty()) return emptyList()
    val ids = favorites.mapNotNull { it.itemId.takeIf { id -> id.isNotEmpty() } }.toSet()
    val names = favorites.map { fold(it.title).trim() }.toSet()
    val order = favorites.withIndex().associate { (i, f) -> fold(f.title).trim() to i }
    return titles.filter { t -> t.items.any { it.id in ids } || fold(t.title).trim() in names }
        .sortedByDescending { order[fold(it.title).trim()] ?: -1 }
}
