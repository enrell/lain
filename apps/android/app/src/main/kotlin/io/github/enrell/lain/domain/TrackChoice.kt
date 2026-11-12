package io.github.enrell.lain.domain

import java.util.Locale

/*
 * Which audio and subtitle tracks the account's preferences pick (D-280):
 * the rule of the server's lain-watch tracks.rs and of the web player,
 * with the same test cases (TrackChoiceTest).
 *
 * - Audio: a track in the audio language, else the file's default.
 * - Subtitles by mode: AUTO shows the subtitle language when the audio that
 *   plays is not in it; ALWAYS shows it over any audio; FORCED takes only a
 *   forced track (subtitle language, else the audio's, else untagged); OFF
 *   takes none. With no subtitle language, or no language tag on any track,
 *   AUTO leaves the player's default alone.
 *
 * A choice made in the player for this title (kept in Prefs) wins.
 */

enum class SubtitleMode(val wire: String) {
    AUTO("auto"), ALWAYS("always"), FORCED("forced"), OFF("off");

    companion object {
        /** An unknown or empty mode is AUTO, as on the server. */
        fun of(wire: String): SubtitleMode = entries.firstOrNull { it.wire == wire.trim().lowercase() } ?: AUTO
    }
}

/** The account's preferences; languages are ISO 639-2, "" for none. */
data class TrackPrefs(val audio: String = "", val subtitle: String = "", val mode: SubtitleMode = SubtitleMode.AUTO) {
    val isEmpty: Boolean get() = audio.isEmpty() && subtitle.isEmpty() && mode == SubtitleMode.AUTO
}

/** What the player chose last time for this title: a language, "" for subtitles off, null for nothing kept. */
data class RememberedTracks(val audio: String? = null, val text: String? = null)

/** One track as the rule sees it: its language tag (any form Media3 or the server writes) and flags. */
data class TrackInfo(val language: String?, val forced: Boolean = false, val default: Boolean = false)

/** The subtitle side of a choice. */
sealed interface TextPick {
    /** Leave the player's own choice. */
    data object Keep : TextPick
    data object Off : TextPick
    data class Track(val index: Int) : TextPick
}

private val BIBLIOGRAPHIC = mapOf(
    "fre" to "fra", "ger" to "deu", "chi" to "zho", "dut" to "nld", "gre" to "ell", "rum" to "ron",
    "cze" to "ces", "slo" to "slk", "per" to "fas", "may" to "msa", "alb" to "sqi", "arm" to "hye",
    "baq" to "eus", "bur" to "mya", "ice" to "isl", "mac" to "mkd",
)

/** A tag as one ISO 639-2/T code: Media3 writes "ja" or "pt-BR" where the server writes "jpn" or "por". */
fun languageCode(tag: String?): String {
    val primary = (tag ?: "").trim().lowercase().substringBefore('-').substringBefore('_')
    if (primary.isEmpty() || primary == "und") return ""
    val three = if (primary.length == 2) runCatching { Locale.forLanguageTag(primary).isO3Language }.getOrDefault(primary) else primary
    return BIBLIOGRAPHIC[three] ?: three
}

fun sameLanguage(actual: String?, preferred: String?): Boolean {
    val a = languageCode(actual)
    return a.isNotEmpty() && a == languageCode(preferred)
}

/** The audio track the preferences pick: in [language], else the default, else the first; null without audio. */
fun chooseAudio(audios: List<TrackInfo>, language: String): Int? {
    if (audios.isEmpty()) return null
    val wanted = if (language.isEmpty()) -1 else audios.indexOfFirst { sameLanguage(it.language, language) }
    if (wanted >= 0) return wanted
    val default = audios.indexOfFirst { it.default }
    return if (default >= 0) default else 0
}

/**
 * The subtitle track the preferences pick from [texts], given the audio that
 * plays. [tagged]: some audio or subtitle track says its language.
 */
fun chooseText(texts: List<TrackInfo>, prefs: TrackPrefs, audioLanguage: String?, tagged: Boolean): TextPick {
    val lang = prefs.subtitle
    fun inLanguage(l: String): Int? {
        val full = texts.indexOfFirst { !it.forced && sameLanguage(it.language, l) }
        if (full >= 0) return full
        return texts.indexOfFirst { sameLanguage(it.language, l) }.takeIf { it >= 0 }
    }
    return when (prefs.mode) {
        SubtitleMode.OFF -> TextPick.Off
        SubtitleMode.FORCED -> {
            fun by(l: String?) = if (l.isNullOrEmpty()) -1 else texts.indexOfFirst { it.forced && sameLanguage(it.language, l) }
            val i = listOf(by(lang), by(audioLanguage), texts.indexOfFirst { it.forced && languageCode(it.language).isEmpty() }).firstOrNull { it >= 0 }
            i?.let { TextPick.Track(it) } ?: TextPick.Off
        }
        SubtitleMode.ALWAYS -> {
            if (lang.isEmpty()) {
                val i = texts.indexOfFirst { !it.forced && it.default }.takeIf { it >= 0 } ?: texts.indexOfFirst { !it.forced }.takeIf { it >= 0 }
                i?.let { TextPick.Track(it) } ?: TextPick.Off
            } else {
                inLanguage(lang)?.let { TextPick.Track(it) } ?: if (tagged) TextPick.Off else TextPick.Keep
            }
        }
        SubtitleMode.AUTO -> when {
            lang.isEmpty() || !tagged -> TextPick.Keep
            sameLanguage(audioLanguage, lang) -> TextPick.Off
            else -> inLanguage(lang)?.let { TextPick.Track(it) } ?: TextPick.Off
        }
    }
}

/**
 * The text decision for one item: what was remembered for the title wins
 * (a language it has, or off), else the account's rule.
 */
fun decideText(texts: List<TrackInfo>, audios: List<TrackInfo>, playingAudio: TrackInfo?, prefs: TrackPrefs, remembered: RememberedTracks): TextPick {
    remembered.text?.let { kept ->
        if (kept.isEmpty()) return TextPick.Off
        val full = texts.indexOfFirst { !it.forced && sameLanguage(it.language, kept) }
        val any = if (full >= 0) full else texts.indexOfFirst { sameLanguage(it.language, kept) }
        if (any >= 0) return TextPick.Track(any)
    }
    if (prefs.isEmpty && remembered.audio == null) return TextPick.Keep
    val tagged = (audios + texts).any { languageCode(it.language).isNotEmpty() }
    return chooseText(texts, prefs, playingAudio?.language, tagged)
}

/**
 * How a player starts, before the file's tracks are known: Media3's own
 * preferences, as close to the rule as they go. [decideText] corrects the
 * subtitles once the tracks are known.
 */
data class StartTracks(
    /** setPreferredAudioLanguage; null leaves the file's default. */
    val audioLanguage: String?,
    /** setPreferredTextLanguage; null leaves Media3's default (the system caption language when captions are on). */
    val textLanguage: String?,
    /** setTrackTypeDisabled(TEXT): subtitles off. */
    val textDisabled: Boolean,
    /** setIgnoredTextSelectionFlags(DEFAULT): only forced tracks qualify (Media3 then takes the one in the audio's language). */
    val forcedOnly: Boolean,
)

fun startTracks(prefs: TrackPrefs, remembered: RememberedTracks): StartTracks {
    val audio = remembered.audio?.takeIf { it.isNotEmpty() } ?: prefs.audio.takeIf { it.isNotEmpty() }
    remembered.text?.let { kept ->
        return StartTracks(audio, kept.takeIf { it.isNotEmpty() }, textDisabled = kept.isEmpty(), forcedOnly = false)
    }
    val lang = prefs.subtitle.takeIf { it.isNotEmpty() }
    return when (prefs.mode) {
        SubtitleMode.OFF -> StartTracks(audio, null, textDisabled = true, forcedOnly = false)
        SubtitleMode.FORCED -> StartTracks(audio, null, textDisabled = false, forcedOnly = true)
        SubtitleMode.ALWAYS, SubtitleMode.AUTO -> StartTracks(audio, lang, textDisabled = false, forcedOnly = false)
    }
}
