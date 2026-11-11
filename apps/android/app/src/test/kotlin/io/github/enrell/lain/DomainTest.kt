package io.github.enrell.lain

import io.github.enrell.lain.domain.Card
import io.github.enrell.lain.domain.RailLabel
import io.github.enrell.lain.domain.buildRails
import io.github.enrell.lain.domain.decodeClaim
import io.github.enrell.lain.domain.fraction
import io.github.enrell.lain.domain.groupTitles
import io.github.enrell.lain.domain.nextUp
import io.github.enrell.lain.domain.skipSeconds
import io.github.enrell.lain.domain.withoutFamily
import io.github.enrell.lain.ui.destinations
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DomainTest {
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
    fun nextUpSkipsFinishedItems() {
        val t = groupTitles(FakeCore().items).first { it.title == "Frieren" }
        assertEquals("e1", nextUp(t, emptyMap()).id)
        assertEquals("e2", nextUp(t, mapOf("e1" to progress("e1", 1440.0, done = true))).id)
        assertEquals("e1", nextUp(t, t.items.associate { it.id to progress(it.id, 1.0, done = true) }).id)
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
