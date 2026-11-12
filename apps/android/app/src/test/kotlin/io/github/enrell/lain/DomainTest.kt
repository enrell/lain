package io.github.enrell.lain

import io.github.enrell.lain.domain.Card
import io.github.enrell.lain.domain.RailLabel
import io.github.enrell.lain.domain.buildRails
import io.github.enrell.lain.domain.decodeClaim
import io.github.enrell.lain.domain.fraction
import io.github.enrell.lain.domain.goOnWith
import io.github.enrell.lain.domain.groupTitles
import io.github.enrell.lain.domain.nextUp
import io.github.enrell.lain.domain.skipSeconds
import io.github.enrell.lain.domain.trackName
import io.github.enrell.lain.domain.withoutFamily
import io.github.enrell.lain.ui.destinations
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DomainTest {
    @Test
    fun trackNamesSayTheLanguageOnce() {
        val en = java.util.Locale.ENGLISH
        // A title that already names the language drops the code beside it.
        assertEquals("English", trackName("English", "en", en))
        assertEquals("Português", trackName("Português", "pt", en))
        // A title that does not keeps it, and codec words go.
        assertEquals("Commentary · EN", trackName("Commentary · AAC", "en", en))
        // No title: the language in the device's words.
        assertEquals("Japanese", trackName(null, "ja", en))
        assertEquals("Signs", trackName("Signs", null, en))
    }

    @Test
    fun titlesGroupByLibraryAndTitleInEpisodeOrder() {
        val items = listOf(
            item("e2", "Frieren", 2, updated = 9),
            item("e1", "frieren", 1, updated = 3),
            item("x", "Frieren", 3, lib = "lib-other"),
            item("gone", "Frieren", 4, missing = true),
            item("a", "Akira", 0),
        )
        val titles = groupTitles(items)
        assertEquals(listOf("Akira", "Frieren", "Frieren"), titles.map { it.title })
        val frieren = titles.first { it.libraryId == "lib-1" && it.title.equals("frieren", true) }
        assertEquals(listOf("e1", "e2"), frieren.items.map { it.id })
        assertEquals(9L, frieren.latest)
        assertFalse(titles.any { t -> t.items.any { it.missing } })
    }

    @Test
    fun finishingAnEpisodePutsTheNextOneOnHome() {
        val items = listOf(item("e1", "Frieren", 1), item("e2", "Frieren", 2), item("e3", "Frieren", 3), item("f", "Akira", 0))
        val titles = groupTitles(items)
        // Episode 1 finished last, Akira half-watched before it: Frieren's next comes first, as a fresh start.
        val done = progress("e1", 100.0, done = true).copy(updatedAt = 20)
        val half = progress("f", 30.0).copy(updatedAt = 10)
        val got = goOnWith(titles, listOf(half), mapOf("e1" to done, "f" to half))
        assertEquals(listOf("e2", "f"), got.map { it.itemId })
        assertEquals(0.0, got[0].positionSec, 0.0)
        // A title with a part half-way keeps that part, not the next one.
        val mid = progress("e3", 5.0).copy(updatedAt = 5)
        assertEquals(listOf("e3"), goOnWith(titles, listOf(mid), mapOf("e1" to done, "e3" to mid)).map { it.itemId })
        // Everything finished: nothing to go on with.
        val all = listOf("e1", "e2", "e3").associateWith { progress(it, 100.0, done = true) }
        assertTrue(goOnWith(titles, emptyList(), all).isEmpty())
    }

    @Test
    fun railsAreContinueRecentAndLibraries() {
        val core = FakeCore()
        val titles = groupTitles(core.items)
        val libs = listOf(io.github.enrell.lain.core.LibraryInfo("lib-1", "Anime", "anime", false, false))
        val cont = listOf(progress("e2", 100.0), progress("e1", 10.0, done = true))
        val rails = buildRails(titles, cont, core.items.associateBy { it.id }, libs)
        assertEquals(listOf(RailLabel.Continue, RailLabel.Recent, RailLabel.Library("Anime")), rails.map { it.label })
        assertEquals("e2", (rails[0].cards.single() as Card.Resume).item.id)
        // Nothing to continue: the rail is dropped.
        assertEquals(RailLabel.Recent, buildRails(titles, emptyList(), emptyMap(), libs).first().label)
    }

    @Test
    fun favoritesNameTitlesAndGetARail() {
        val core = FakeCore()
        val titles = groupTitles(core.items)
        val frieren = titles.first { it.title == "Frieren" }
        // By any item of the title, or by its name alone (an item that left the catalog).
        val byItem = listOf(io.github.enrell.lain.core.FavoriteInfo(frieren.items.last().id, "anime", "whatever"))
        val byName = listOf(io.github.enrell.lain.core.FavoriteInfo("", "anime", "  frieren "))
        assertEquals(listOf(frieren.key), io.github.enrell.lain.domain.favoriteTitles(titles, byItem).map { it.key })
        assertEquals(listOf(frieren.key), io.github.enrell.lain.domain.favoriteTitles(titles, byName).map { it.key })
        assertTrue(io.github.enrell.lain.domain.favoriteTitles(titles, emptyList()).isEmpty())
        val libs = listOf(io.github.enrell.lain.core.LibraryInfo("lib-1", "Anime", "anime", false, false))
        val rails = buildRails(titles, emptyList(), core.items.associateBy { it.id }, libs, listOf(frieren))
        assertTrue(rails.any { it.label == RailLabel.MyList && it.cards.size == 1 })
    }

    @Test
    fun nextUpSkipsFinishedItems() {
        val t = groupTitles(FakeCore().items).first { it.title == "Frieren" }
        assertEquals("e1", nextUp(t, emptyMap()).id)
        assertEquals("e2", nextUp(t, mapOf("e1" to progress("e1", 1440.0, done = true))).id)
        assertEquals("e1", nextUp(t, t.items.associate { it.id to progress(it.id, 1.0, done = true) }).id)
        // The one in progress wins; otherwise what follows the last one finished.
        if (t.items.size >= 3) {
            assertEquals(t.items[2].id, nextUp(t, mapOf(t.items[1].id to progress(t.items[1].id, 1440.0, done = true))).id)
            assertEquals(t.items[0].id, nextUp(t, mapOf(t.items[1].id to progress(t.items[1].id, 1440.0, done = true), t.items[0].id to progress(t.items[0].id, 300.0))).id)
        }
        assertEquals(0.5f, fraction(progress("e1", 720.0)), 1e-6f)
        assertEquals(1f, fraction(progress("e1", 0.0, done = true)), 0f)
        assertEquals(0f, fraction(progress("e1", 10.0, dur = 0.0)), 0f)
    }

    @Test
    fun decodeClaimUsesTheServerVocabulary() {
        val claim = decodeClaim(setOf("h264", "aac"))
        assertTrue("mp4" in claim && "mp4/h264" in claim && "mkv/aac" in claim)
        assertFalse(claim.any { it.startsWith("webm") || it.startsWith("ogg") })
        assertEquals(claim.sorted(), claim)
        assertTrue(decodeClaim(emptySet()).isEmpty())
        assertTrue("webm/vp9" in decodeClaim(setOf("vp9")))
    }

    @Test
    fun destinationsFollowEnabledModules() {
        val core = FakeCore()
        assertEquals(listOf("home", "library", "search", "downloads", "settings"), destinations(core.modules).map { it.id })
        val withRead = core.modules.map { if (it.id == "read") it.copy(enabled = true) else it }
        // Comics are libraries, reached like the others: reading adds no place of its own.
        assertEquals(listOf("home", "library", "search", "downloads", "settings"), destinations(withRead).map { it.id })
        // /api/modules unavailable: the core destinations.
        assertEquals(listOf("home", "library", "search", "downloads", "settings"), destinations(null).map { it.id })
    }

    @Test
    fun withoutFamilyDropsTheCodecAndEmptyContainers() {
        val claim = decodeClaim(setOf("hevc", "aac", "vp9"))
        val out = withoutFamily(claim, "hevc")
        assertFalse(out.any { it.endsWith("/hevc") })
        assertTrue("mkv/aac" in out && "webm/vp9" in out)
        assertEquals(listOf("webm", "webm/vp9"), withoutFamily(decodeClaim(setOf("vp9", "hevc")), "hevc").filter { it.startsWith("webm") })
        assertTrue(withoutFamily(decodeClaim(setOf("hevc")), "hevc").isEmpty())
    }

    @Test
    fun aSkipIsLongerInALongTitleUnlessChosen() {
        assertEquals(10, skipSeconds(0, 24 * 60.0))
        assertEquals(30, skipSeconds(0, 2 * 3600.0 + 10))
        assertEquals(10, skipSeconds(0, 0.0))
        assertEquals(60, skipSeconds(60, 24 * 60.0))
        assertEquals(5, skipSeconds(5, 3 * 3600.0))
    }
}
