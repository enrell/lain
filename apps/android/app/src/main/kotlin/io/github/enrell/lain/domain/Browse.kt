package io.github.enrell.lain.domain

import io.github.enrell.lain.core.ProgressInfo
import java.text.Normalizer

/*
 * Searching, sorting and narrowing a library: the same rules as the web
 * client (web/src/lib/utilities/browse.ts), so a filter gives the same
 * titles in both.
 */

enum class Sort(val id: String) {
    Title("title"), TitleDesc("title_desc"), Recent("recent"), Oldest("oldest"),
    YearDesc("year_desc"), Year("year"), SizeDesc("size_desc"), EpisodesDesc("episodes_desc");

    companion object {
        fun of(id: String): Sort = entries.firstOrNull { it.id == id } ?: Title
    }
}

enum class Watch { Unwatched, InProgress, Watched }

enum class Form { Series, Single }

data class Filters(
    val q: String = "",
    val sort: Sort = Sort.Title,
    val watch: Watch? = null,
    val form: Form? = null,
    val kind: String = "",
    /** Days back, 0 for any. */
    val added: Int = 0,
    /** Genres a title must all carry. */
    val genres: List<String> = emptyList(),
) {
    val active: Int get() = listOf(watch != null, form != null, kind.isNotEmpty(), added > 0).count { it } + genres.size
    val narrowed: Boolean get() = active > 0 || q.isNotBlank()
    fun cleared(): Filters = Filters(sort = sort)
}

val ADDED_DAYS = listOf(1, 7, 30, 90)

/** Lower case, accents folded, so "pokemon" finds "Pokémon". */
fun fold(text: String): String =
    Normalizer.normalize(text, Normalizer.Form.NFD).replace(Regex("\\p{Mn}+"), "").lowercase()

fun watchState(title: Title, progress: Map<String, ProgressInfo>): Watch {
    var done = 0
    var started = 0
    for (item in title.items) {
        val p = progress[item.id] ?: continue
        if (p.completed) done++ else if (p.positionSec > 0) started++
    }
    return when {
        done == title.items.size -> Watch.Watched
        done + started > 0 -> Watch.InProgress
        else -> Watch.Unwatched
    }
}

enum class Axis { Watch, Form, Kind, Added, Genres }

/** A title's genres, however the caller knows them (the overlays). */
typealias GenresOf = (Title) -> List<String>

private val NO_GENRES: GenresOf = { emptyList() }

/** Whether a title passes; [skip] leaves one axis out (how a facet counts its options). */
fun matches(t: Title, f: Filters, progress: Map<String, ProgressInfo>, now: Long, skip: Axis? = null, genres: GenresOf = NO_GENRES): Boolean {
    if (f.q.isNotBlank()) {
        val hay = fold(t.title)
        if (fold(f.q).split(Regex("\\s+")).filter { it.isNotEmpty() }.any { it !in hay }) return false
    }
    if (skip != Axis.Form && f.form != null && (f.form == Form.Series) != (t.items.size > 1)) return false
    if (skip != Axis.Kind && f.kind.isNotEmpty() && t.items.none { it.kind == f.kind }) return false
    if (skip != Axis.Added && f.added > 0 && t.latest < now - f.added * 86400L) return false
    if (skip != Axis.Watch && f.watch != null && watchState(t, progress) != f.watch) return false
    if (skip != Axis.Genres && f.genres.isNotEmpty()) {
        val have = genres(t).map { it.lowercase() }.toSet()
        if (f.genres.any { it.lowercase() !in have }) return false
    }
    return true
}

private val byTitle = Comparator<Title> { a, b -> String.CASE_INSENSITIVE_ORDER.compare(a.title, b.title) }
private fun size(t: Title): Long = t.items.sumOf { it.size }

fun sorted(titles: List<Title>, sort: Sort): List<Title> = when (sort) {
    Sort.Title -> titles.sortedWith(byTitle)
    Sort.TitleDesc -> titles.sortedWith(byTitle.reversed())
    Sort.Recent -> titles.sortedWith(compareByDescending<Title> { it.latest }.then(byTitle))
    Sort.Oldest -> titles.sortedWith(compareBy<Title> { it.latest }.then(byTitle))
    // A title with no year sorts last either way.
    Sort.YearDesc -> titles.sortedWith(compareByDescending<Title> { it.year }.then(byTitle))
    Sort.Year -> titles.sortedWith(compareBy<Title> { if (it.year == 0L) 9999L else it.year }.then(byTitle))
    Sort.SizeDesc -> titles.sortedWith(compareByDescending<Title> { size(it) }.then(byTitle))
    Sort.EpisodesDesc -> titles.sortedWith(compareByDescending<Title> { it.items.size }.then(byTitle))
}

fun applyFilters(titles: List<Title>, f: Filters, progress: Map<String, ProgressInfo>, now: Long, genres: GenresOf = NO_GENRES): List<Title> =
    sorted(titles.filter { matches(it, f, progress, now, genres = genres) }, f.sort)

/** Genres with the titles each would leave if picked on top of the other filters, most common first. */
fun genreFacets(titles: List<Title>, f: Filters, progress: Map<String, ProgressInfo>, now: Long, genres: GenresOf): List<Pair<String, Int>> {
    val counts = LinkedHashMap<String, Pair<String, Int>>()
    for (g in f.genres) counts[g.lowercase()] = g to 0
    for (t in titles) {
        if (!matches(t, f, progress, now, genres = genres)) continue
        for (g in genres(t)) counts[g.lowercase()] = (counts[g.lowercase()]?.first ?: g) to ((counts[g.lowercase()]?.second ?: 0) + 1)
    }
    return counts.values.sortedWith(compareByDescending<Pair<String, Int>> { it.second }.thenBy { it.first })
}

/** Counts per option of one axis, ignoring that axis's own pick. */
fun <K> countBy(titles: List<Title>, f: Filters, progress: Map<String, ProgressInfo>, now: Long, axis: Axis, genres: GenresOf = NO_GENRES, values: (Title) -> List<K>): Map<K, Int> {
    val out = HashMap<K, Int>()
    for (t in titles) {
        if (!matches(t, f, progress, now, axis, genres)) continue
        for (v in values(t)) out[v] = (out[v] ?: 0) + 1
    }
    return out
}
