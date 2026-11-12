package io.github.enrell.lain

import io.github.enrell.lain.core.AudioTrack
import io.github.enrell.lain.core.PlaySource
import io.github.enrell.lain.core.SourceKind
import io.github.enrell.lain.domain.RememberedTracks
import io.github.enrell.lain.domain.TrackPrefs
import io.github.enrell.lain.domain.audioIn
import io.github.enrell.lain.domain.audioNeedsSession
import io.github.enrell.lain.domain.sessionAudio
import io.github.enrell.lain.domain.shortLanguage
import io.github.enrell.lain.domain.transcodeAudioLanguage
import io.github.enrell.lain.ui.PlayerModel
import io.github.enrell.lain.ui.screens.streamAudioOf
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

/**
 * D-280 on a transcode: an HLS session carries one audio stream, so the
 * title's audio is asked for as the session starts and another track is a
 * new session; a direct play switches inside the file and asks for nothing.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class TranscodeAudioTest {
    private val main = UnconfinedTestDispatcher()

    @Before
    fun setUp() = Dispatchers.setMain(main)

    @After
    fun tearDown() = Dispatchers.resetMain()

    /** English dub first and default, Japanese second, as the server tags them. */
    private val anime = listOf(AudioTrack(1, "eng", "eng · English · AAC", true), AudioTrack(2, "jpn", "jpn · Japanese · AAC", false))
    private val english = TrackPrefs(audio = "eng", subtitle = "por")

    private fun hls(asked: Long? = null, session: String = "sess-1") =
        PlaySource(SourceKind.HLS, "http://s/hls", "transcode", 0.0, 0.0, 1440.0, emptyList(), session, anime, asked)

    private fun direct() = PlaySource(SourceKind.DIRECT, "http://s/stream", "direct play", 0.0, 0.0, 1440.0, emptyList(), "", anime, null)

    @Test
    fun aTranscodeStartsInTheTitlesAudioOnlyWhenTheAccountWouldNotPickIt() {
        // Media3 remembered "ja"; the core is sent the server's code.
        assertEquals("jpn", transcodeAudioLanguage(english, RememberedTracks(audio = "ja")))
        assertEquals("fra", transcodeAudioLanguage(english, RememberedTracks(audio = "fre")))
        // The account's language picks it already: the server's choice, and the session the plan prepared.
        assertEquals("", transcodeAudioLanguage(english, RememberedTracks(audio = "en")))
        assertEquals("", transcodeAudioLanguage(english, RememberedTracks()))
        assertEquals("", transcodeAudioLanguage(english, RememberedTracks(audio = "")))
        assertEquals("jpn", transcodeAudioLanguage(TrackPrefs(), RememberedTracks(audio = "jpn")))
    }

    @Test
    fun theSessionsAudioIsTheOneAskedForElseTheServersChoice() {
        assertEquals(1L, sessionAudio(anime, null, english))
        assertEquals(2L, sessionAudio(anime, null, TrackPrefs(audio = "jpn")))
        // No language, or one the file lacks: the file's default.
        assertEquals(1L, sessionAudio(anime, null, TrackPrefs(audio = "kor")))
        assertEquals(2L, sessionAudio(anime, 2, english))
        assertNull(sessionAudio(emptyList(), null, english))
        assertEquals(2L, audioIn(anime, "ja"))
        assertNull(audioIn(anime, ""))
        assertNull(audioIn(anime, "kor"))
    }

    @Test
    fun onlyAnotherStreamOfAnHlsSessionNeedsANewSession() {
        assertTrue(audioNeedsSession(hls(), 2, english))
        assertFalse("it plays already", audioNeedsSession(hls(), 1, english))
        assertFalse("it was asked for", audioNeedsSession(hls(asked = 2), 2, english))
        assertTrue(audioNeedsSession(hls(asked = 2), 1, english))
        assertFalse("not one of the file's streams", audioNeedsSession(hls(), 7, english))
        assertFalse("Media3 switches a direct play itself", audioNeedsSession(direct(), 2, english))
        assertFalse(audioNeedsSession(direct().copy(kind = SourceKind.OFFLINE), 2, english))
    }

    @Test
    fun anHlsSessionListsTheFilesAudioWithTheOneItPlays() {
        val list = streamAudioOf(hls(asked = 2), english)
        assertEquals(listOf(1L to false, 2L to true), list.map { it.stream to it.selected })
        assertEquals(listOf("English", "Japanese"), list.map { it.label })
        assertEquals(listOf(1L to true, 2L to false), streamAudioOf(hls(), english).map { it.stream to it.selected })
        // A direct play lists Media3's tracks; one stream is no choice.
        assertTrue(streamAudioOf(direct(), english).isEmpty())
        assertTrue(streamAudioOf(hls().copy(audio = anime.take(1)), english).isEmpty())
        assertTrue(streamAudioOf(null, english).isEmpty())
        assertEquals(listOf("ja", "fr", "pt", "und"), listOf("jpn", "fre", "por", "und").map(::shortLanguage))
    }

    @Test
    fun pickingAnotherAudioOnATranscodeAsksForASessionWithIt() = runTest(main) {
        val core = FakeCore().apply { source = hls() }
        val m = PlayerModel(core, "e1", emptyList(), this, trackPrefs = english)
        m.load()
        advanceUntilIdle()
        assertTrue(core.transcodes.isEmpty())

        assertTrue(m.switchAudio(2, 90.0))
        advanceUntilIdle()
        assertEquals(FakeCore.Transcode(90.0, "", null, 2), core.transcodes.last())
        val src = m.state.value.source!!
        assertEquals(2L, src.audioStream)
        assertEquals("the file's list carries over", anime, src.audio)
        assertEquals(90.0, src.offsetSec, 0.001)
        assertEquals(listOf("sess-1"), core.stopped)

        // The stream that plays, or a seek: no audio change.
        assertFalse(m.switchAudio(2, 95.0))
        assertEquals(1, core.transcodes.size)
        m.rebuild(600.0)
        advanceUntilIdle()
        assertEquals("a seek keeps the picked audio", FakeCore.Transcode(600.0, "", null, 2), core.transcodes.last())
        m.rebuild(600.0, quality = "720p")
        advanceUntilIdle()
        assertEquals("so does a quality change", FakeCore.Transcode(600.0, "720p", null, 2), core.transcodes.last())
    }

    @Test
    fun aDirectPlaySwitchesItselfAndAsksForNothing() = runTest(main) {
        val core = FakeCore().apply { source = direct() }
        val m = PlayerModel(core, "e1", emptyList(), this, trackPrefs = english)
        m.load()
        advanceUntilIdle()
        assertFalse(m.switchAudio(2, 90.0))
        assertTrue(core.transcodes.isEmpty())
        assertEquals(SourceKind.DIRECT, m.state.value.source!!.kind)
    }

    @Test
    fun theTitlesAudioGoesWithEveryStart() = runTest(main) {
        val core = FakeCore().apply { source = hls(asked = 2) }
        var language = "jpn"
        val m = PlayerModel(core, "e1", listOf("mkv", "mkv/h264", "mkv/hevc"), this, trackPrefs = english, audioLanguage = { language })
        m.load()
        advanceUntilIdle()
        // The core named the stream: the first session is the one, no second asked.
        assertEquals(listOf("jpn"), core.playLanguages)
        assertTrue(core.transcodes.isEmpty())
        // A direct play whose decoder gave up becomes a session in it too.
        core.source = direct()
        m.original(30.0)
        advanceUntilIdle()
        core.source = hls(asked = 2)
        assertTrue(m.decoderFailed("video/hevc", 0, 0))
        advanceUntilIdle()
        assertEquals(listOf("jpn", "jpn", "jpn"), core.playLanguages)
        assertEquals(2L, m.state.value.source!!.audioStream)
        // Read at each start: a title with nothing kept leaves the server's choice.
        language = ""
        m.original(30.0)
        advanceUntilIdle()
        assertEquals("", core.playLanguages.last())
    }

    @Test
    fun aSessionNotInTheTitlesAudioIsAskedAgain() = runTest(main) {
        // The core could not name the stream (a tag it does not fold): this side finds it and asks once.
        val core = FakeCore().apply { source = hls().copy(audio = listOf(AudioTrack(1, "eng", "", true), AudioTrack(2, "ja", "", false))) }
        val m = PlayerModel(core, "e1", emptyList(), this, trackPrefs = english, audioLanguage = { "jpn" })
        m.load()
        advanceUntilIdle()
        assertEquals(listOf(FakeCore.Transcode(0.0, "", null, 2)), core.transcodes)
        assertEquals(listOf("sess-1"), core.stopped)
        assertEquals(2L, m.state.value.source!!.audioStream)
        assertEquals(2, m.state.value.source!!.audio.size)
    }

    @Test
    fun aMeteredStartCarriesTheTitlesAudio() = runTest(main) {
        val core = FakeCore().apply { source = direct() }
        val m = PlayerModel(core, "e1", emptyList(), this, trackPrefs = english, audioLanguage = { "jpn" })
        m.load(quality = "480p")
        advanceUntilIdle()
        assertEquals(listOf(FakeCore.Transcode(0.0, "480p", null, 2)), core.transcodes)
        assertEquals(SourceKind.HLS, m.state.value.source!!.kind)
        // Without a language of its own the server chooses by the account's.
        val plain = FakeCore().apply { source = direct() }
        PlayerModel(plain, "e1", emptyList(), this, trackPrefs = english).load(quality = "480p")
        advanceUntilIdle()
        assertEquals(listOf(FakeCore.Transcode(0.0, "480p", null, null)), plain.transcodes)
    }
}
