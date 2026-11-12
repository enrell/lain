package io.github.enrell.lain

import io.github.enrell.lain.domain.RememberedTracks
import io.github.enrell.lain.domain.StartTracks
import io.github.enrell.lain.domain.SubtitleMode
import io.github.enrell.lain.domain.TextPick
import io.github.enrell.lain.domain.TrackInfo
import io.github.enrell.lain.domain.TrackPrefs
import io.github.enrell.lain.domain.chooseAudio
import io.github.enrell.lain.domain.decideText
import io.github.enrell.lain.domain.sameLanguage
import io.github.enrell.lain.domain.startTracks
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The cases of lain-watch's tracks.rs (D-280), on Media3's language tags ("ja", "pt-BR"). */
class TrackChoiceTest {
    private fun p(audio: String, subtitle: String, mode: SubtitleMode = SubtitleMode.AUTO) = TrackPrefs(audio, subtitle, mode)

    /** English dub first and default, Japanese second. */
    private val animeAudio = listOf(TrackInfo("en", default = true), TrackInfo("ja"))

    /** Signs, then full Portuguese and English subtitles. */
    private val animeText = listOf(TrackInfo("en", forced = true), TrackInfo("pt-BR"), TrackInfo("en"))

    /** The rule as the player runs it: the audio Media3 picks, then the subtitles. */
    private fun pick(audios: List<TrackInfo>, texts: List<TrackInfo>, prefs: TrackPrefs, remembered: RememberedTracks = RememberedTracks()): Pair<Int?, TextPick> {
        val audio = chooseAudio(audios, remembered.audio ?: prefs.audio)
        return audio to decideText(texts, audios, audio?.let { audios[it] }, prefs, remembered)
    }

    @Test
    fun theOwnersCaseJapaneseAudioPortugueseSubtitles() {
        assertEquals(1 to TextPick.Track(1), pick(animeAudio, animeText, p("jpn", "por")))
        // A film dubbed in Portuguese needs no subtitles.
        assertEquals(0 to TextPick.Off, pick(listOf(TrackInfo("pt")), listOf(TrackInfo("en")), p("jpn", "por")))
        // English subtitles when the release has no Portuguese ones: the full track, not the signs.
        assertEquals(1 to TextPick.Track(2), pick(animeAudio, animeText, p("jpn", "eng")))
    }

    @Test
    fun oneLanguageIsD071() {
        val por = p("por", "por")
        assertEquals(0 to TextPick.Off, pick(listOf(TrackInfo("pt")), listOf(TrackInfo("en")), por))
        assertEquals(0 to TextPick.Track(0), pick(listOf(TrackInfo("ja")), listOf(TrackInfo("pt")), por))
        assertEquals(0 to TextPick.Off, pick(listOf(TrackInfo("ja")), listOf(TrackInfo("en")), por))
        assertEquals(0 to TextPick.Keep, pick(listOf(TrackInfo(null)), listOf(TrackInfo("und")), por))
        assertEquals(0 to TextPick.Keep, pick(animeAudio, animeText, TrackPrefs()))
    }

    @Test
    fun modes() {
        val dubbed = listOf(TrackInfo("pt")) to listOf(TrackInfo("en"))
        assertEquals(0 to TextPick.Track(0), pick(dubbed.first, dubbed.second, p("", "eng", SubtitleMode.ALWAYS)))
        assertEquals(0 to TextPick.Off, pick(dubbed.first, dubbed.second, p("", "por", SubtitleMode.ALWAYS)))
        val flagged = listOf(TrackInfo("en", forced = true), TrackInfo("en"), TrackInfo("es", default = true))
        assertEquals(0 to TextPick.Track(2), pick(listOf(TrackInfo("ja")), flagged, p("", "", SubtitleMode.ALWAYS)))
        assertEquals(0 to TextPick.Track(0), pick(animeAudio, animeText, p("", "eng", SubtitleMode.FORCED)))
        assertEquals(0 to TextPick.Track(0), pick(animeAudio, animeText, p("", "", SubtitleMode.FORCED)))
        assertEquals(1 to TextPick.Off, pick(animeAudio, animeText, p("jpn", "por", SubtitleMode.FORCED)))
        assertEquals(0 to TextPick.Track(0), pick(listOf(TrackInfo("ja")), listOf(TrackInfo(null, forced = true)), p("", "por", SubtitleMode.FORCED)))
        assertEquals(1 to TextPick.Off, pick(animeAudio, animeText, p("jpn", "por", SubtitleMode.OFF)))
    }

    @Test
    fun languagesFoldMedia3TagsAndBibliographicCodes() {
        assertTrue(sameLanguage("ja", "jpn") && sameLanguage("pt-BR", "por") && sameLanguage("fre", "fra") && sameLanguage("fr", "fre"))
        assertFalse(sameLanguage("und", "und") || sameLanguage(null, "") || sameLanguage("en", "por"))
        assertEquals(0, chooseAudio(animeAudio, "kor"))
        assertEquals(null, chooseAudio(emptyList(), "jpn"))
    }

    @Test
    fun thisTitlesRememberedChoiceWins() {
        assertEquals(0 to TextPick.Track(1), pick(animeAudio, animeText, p("jpn", "por"), RememberedTracks(audio = "en")))
        assertEquals(1 to TextPick.Off, pick(animeAudio, animeText, p("jpn", "por"), RememberedTracks(text = "")))
        assertEquals(1 to TextPick.Track(2), pick(animeAudio, animeText, p("jpn", "por"), RememberedTracks(text = "en")))
        assertEquals(1 to TextPick.Track(1), pick(animeAudio, animeText, TrackPrefs(), RememberedTracks(audio = "ja", text = "pt")))
        // A language the file lacks falls back to the account's rule.
        assertEquals(1 to TextPick.Track(1), pick(animeAudio, animeText, p("jpn", "por"), RememberedTracks(text = "ko")))
    }

    @Test
    fun media3StartsAsCloseToTheRuleAsItsPreferencesGo() {
        assertEquals(StartTracks("jpn", "por", textDisabled = false, forcedOnly = false), startTracks(p("jpn", "por"), RememberedTracks()))
        assertEquals(StartTracks("jpn", null, textDisabled = true, forcedOnly = false), startTracks(p("jpn", "por", SubtitleMode.OFF), RememberedTracks()))
        assertEquals(StartTracks(null, null, textDisabled = false, forcedOnly = true), startTracks(p("", "por", SubtitleMode.FORCED), RememberedTracks()))
        assertEquals(StartTracks(null, null, textDisabled = false, forcedOnly = false), startTracks(TrackPrefs(), RememberedTracks()))
        // What the title kept wins: its audio, and its subtitles or none.
        assertEquals(StartTracks("en", null, textDisabled = true, forcedOnly = false), startTracks(p("jpn", "por"), RememberedTracks("en", "")))
        assertEquals(StartTracks("jpn", "pt", textDisabled = false, forcedOnly = false), startTracks(p("jpn", "eng", SubtitleMode.OFF), RememberedTracks(text = "pt")))
        assertEquals(SubtitleMode.FORCED, SubtitleMode.of(" Forced "))
        assertEquals(SubtitleMode.AUTO, SubtitleMode.of(""))
    }

    @Test
    fun theSettingsSaveThroughTheCore() = runTest {
        val core = FakeCore()
        val u = core.setTrackPreferences("jpn", "por", "always")
        assertEquals(Triple("jpn", "por", "always"), Triple(u.preferredAudioLanguage, u.preferredSubtitleLanguage, u.preferredSubtitleMode))
        assertEquals(p("jpn", "por", SubtitleMode.ALWAYS), io.github.enrell.lain.ui.screens.trackPrefsOf(u))
        assertEquals(TrackPrefs(), io.github.enrell.lain.ui.screens.trackPrefsOf(null))
    }
}
