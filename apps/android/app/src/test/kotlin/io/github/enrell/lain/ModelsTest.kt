package io.github.enrell.lain

import io.github.enrell.lain.core.CoreEvent
import io.github.enrell.lain.core.LainException
import io.github.enrell.lain.core.PlaySource
import io.github.enrell.lain.core.SourceKind
import io.github.enrell.lain.core.SubtitleTrack
import io.github.enrell.lain.ui.AppViewModel
import io.github.enrell.lain.ui.Phase
import io.github.enrell.lain.ui.PlayerModel
import io.github.enrell.lain.ui.ReaderModel
import io.github.enrell.lain.ui.Screen
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ModelsTest {
    private val main = UnconfinedTestDispatcher()

    @Before
    fun setUp() = Dispatchers.setMain(main)

    @After
    fun tearDown() = Dispatchers.resetMain()

    // ---- the app: session, navigation, catalog ------------------------------

    @Test
    fun connectLoginAndLoadTheCatalog() = runTest(main) {
        val core = FakeCore()
        var remembered: String? = null
        val vm = AppViewModel(core, rememberServer = { remembered = it }, io = main)
        vm.start(null)
        assertEquals(Phase.Connect, vm.state.value.phase)

        vm.connect("192.168.1.10:9360/")
        assertEquals(Phase.Login, vm.state.value.phase)
        assertEquals("http://192.168.1.10:9360", remembered)

        vm.login("bob", "nope")
        assertTrue(vm.state.value.error is LainException.LoginFailed)
        assertEquals(Phase.Login, vm.state.value.phase)

        core.progress["e1"] = progress("e1", 300.0)
        vm.login("bob", core.password)
        val s = vm.state.value
        assertEquals(Phase.Main, s.phase)
        assertEquals("bob", s.user?.username)
        assertEquals(listOf("home", "library", "search", "downloads", "settings"), s.destinations.map { it.id })
        assertEquals(2, s.titles.size)
        assertEquals(listOf("e1"), s.continues.map { it.itemId })
        assertFalse(s.fromCache)
        assertTrue(s.online)
        core.stopEvents()
    }

    @Test
    fun aSavedSessionGoesStraightHomeAndWorksOffline() = runTest(main) {
        val core = FakeCore().apply {
            sessionInfo = io.github.enrell.lain.core.SessionInfo("http://s", "user-bob", "bob")
            offline = true
        }
        val vm = AppViewModel(core, io = main)
        vm.start("http://s")
        val s = vm.state.value
        assertEquals(Phase.Main, s.phase)
        assertTrue("the catalog comes from the device snapshot", s.fromCache)
        assertFalse(s.online)
        assertEquals(2, s.titles.size)
        // The modules route is unreachable: the core destinations stay.
        assertEquals("home", s.destinations.first().id)

        // Back online: the stream connects and the app refreshes.
        core.offline = false
        core.events.trySend(CoreEvent.Connected(1))
        advanceUntilIdle()
        assertTrue(vm.state.value.online)
        assertFalse(vm.state.value.fromCache)
        core.stopEvents()
    }

    @Test
    fun aRefusedSessionReturnsToLogin() = runTest(main) {
        val core = FakeCore().apply { sessionInfo = io.github.enrell.lain.core.SessionInfo("http://s", "u", "bob") }
        val vm = AppViewModel(core, io = main)
        vm.start("http://s")
        core.events.trySend(CoreEvent.AuthRequired)
        advanceUntilIdle()
        assertEquals(Phase.Login, vm.state.value.phase)
    }

    @Test
    fun navigationStackAndOpen() = runTest(main) {
        val core = FakeCore().apply { sessionInfo = io.github.enrell.lain.core.SessionInfo("http://s", "u", "bob") }
        val vm = AppViewModel(core, io = main)
        vm.start("http://s")
        val s = vm.state.value
        vm.go(Screen.TitleDetail(s.titles.first { it.title == "Frieren" }.key))
        vm.open(core.items.first { it.id == "e1" })
        assertEquals(Screen.Player("e1"), vm.state.value.screen)
        assertTrue(vm.back())
        assertTrue(vm.state.value.screen is Screen.TitleDetail)
        vm.open(core.items.first { it.kind == "manga" })
        assertEquals(Screen.Reader("m1"), vm.state.value.screen)
        // A destination resets the stack; back from a destination goes Home, then out.
        vm.go(Screen.Library)
        assertEquals(listOf<Screen>(Screen.Library), vm.state.value.stack)
        assertTrue(vm.back())
        assertEquals(Screen.Home, vm.state.value.screen)
        assertFalse(vm.back())
        vm.search("frier")
        advanceUntilIdle()
        assertEquals(2, vm.state.value.results.size)
        core.stopEvents()
    }

    // ---- the player ----------------------------------------------------------

    @Test
    fun playerResumesAndSavesInSourceTime() = runTest(main) {
        val core = FakeCore()
        core.progress["e1"] = progress("e1", 600.0)
        val m = PlayerModel(core, "e1", listOf("mp4"), this)
        m.load()
        val src = m.state.value.source!!
        assertEquals(600_000L, m.seekOnStartMs(src))

        m.report(700_000, 1_440_000)?.join()
        assertEquals(700.0, core.saved.last().positionSec, 0.001)
        assertFalse(core.saved.last().completed)

        // Past 92 % it counts as watched.
        m.report(1_400_000, 1_440_000)?.join()
        assertTrue(core.saved.last().completed)
        assertTrue(m.state.value.completed)
    }

    @Test
    fun anHlsSessionFromTheResumePointIsOffset() = runTest(main) {
        val core = FakeCore()
        core.progress["e1"] = progress("e1", 600.0)
        core.source = PlaySource(SourceKind.HLS, "http://s/hls", "transcode", 0.0, 600.0, 1440.0, emptyList(), "sess-1")
        val m = PlayerModel(core, "e1", emptyList(), this)
        m.load()
        val src = m.state.value.source!!
        assertEquals(0L, m.seekOnStartMs(src.copy(startSec = 600.0)))
        m.report(30_000, 840_000)?.join()
        assertEquals(630.0, core.saved.last().positionSec, 0.001)
        m.close(31_000, 840_000).join()
        assertEquals(listOf("sess-1"), core.stopped)
    }

    @Test
    fun offlineSavesAreQueuedAndShown() = runTest(main) {
        val core = FakeCore().apply { offline = true }
        val m = PlayerModel(core, "e1", emptyList(), this)
        m.load()
        m.report(120_000, 1_440_000)?.join()
        assertTrue(m.state.value.savedLocally)
        // Under a second of play is not worth a record.
        val before = core.saved.size
        assertNull(m.report(500, 1_440_000))
        assertEquals(before, core.saved.size)
    }

    @Test
    fun typesetSubtitlesBurnIn() = runTest(main) {
        val core = FakeCore()
        val ass = SubtitleTrack("http://s/sub", "jpn · ASS", "jpn", "embedded", false, true, 3)
        val srt = SubtitleTrack("http://s/sc", "eng", "eng", "sidecar", false, false, -1)
        core.source = core.source.copy(subtitles = listOf(srt, ass))
        val m = PlayerModel(core, "e1", emptyList(), this)
        m.load()
        assertEquals(listOf(ass), m.burnable())
        m.burnIn(ass, 90_000)
        val src = m.state.value.source!!
        assertEquals(SourceKind.HLS, src.kind)
        assertEquals(90.0, src.offsetSec, 0.001)
        assertEquals("the plan's length is kept", 1440.0, src.durationSec!!, 0.001)
    }

    // ---- the reader ----------------------------------------------------------

    @Test
    fun readerResumesAtTheSavedPageAndSavesTurns() = runTest(main) {
        val core = FakeCore()
        core.progress["m1"] = progress("m1", 3.0, dur = 5.0)
        val m = ReaderModel(core, "m1", this)
        m.load()
        val s = m.state.value
        assertEquals(2, s.page)
        assertTrue(s.rtl)
        assertEquals(5, s.count)

        m.next()
        advanceUntilIdle()
        assertEquals(4.0, core.saved.last().positionSec, 0.0)
        m.next()
        advanceUntilIdle()
        assertTrue("the last page finishes the book", core.saved.last().completed)
        m.next()
        assertEquals(4, m.state.value.page)
        m.goTo(-3)
        assertEquals(0, m.state.value.page)
        assertEquals(byteArrayOf(0).toList(), m.page(0)!!.toList())
        m.close()?.join()
        assertEquals(1.0, core.saved.last().positionSec, 0.0)
    }

    @Test
    fun aFinishedBookStartsOver() = runTest(main) {
        val core = FakeCore()
        core.progress["m1"] = progress("m1", 5.0, dur = 5.0, done = true)
        val m = ReaderModel(core, "m1", this)
        m.load()
        assertEquals(0, m.state.value.page)
    }
}
