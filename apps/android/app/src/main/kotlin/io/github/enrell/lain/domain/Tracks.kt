package io.github.enrell.lain.domain

import java.util.Locale

/** Codec and format words a track title carries that say nothing to a viewer. */
private val TRACK_NOISE = setOf("subrip", "srt", "ass", "ssa", "webvtt", "vtt", "mov_text", "pgs", "aac", "ac3", "eac3", "opus", "flac")

/**
 * A track's name for the player's lists: the words of its title, then its
 * language code only when no word already names that language, so a track
 * titled "English" reads "English", not "English · EN". A track with no
 * title is named by its language in the device's words ("Portuguese"),
 * and by its code only when the language has no known name.
 */
fun trackName(label: String?, language: String?, display: Locale = Locale.getDefault()): String {
    val lang = language?.takeIf { it.isNotBlank() && it != "und" }
    val words = (label ?: "").split(" · ", " - ").map { it.trim() }
        .filter { w -> w.isNotEmpty() && w.lowercase() !in TRACK_NOISE && !w.equals(lang, ignoreCase = true) && !(w.length == 3 && w == w.lowercase()) }
        .distinctBy { it.lowercase() }
    if (lang == null) return words.joinToString(" · ")
    val locale = Locale.forLanguageTag(lang)
    // Its names in English, in itself and in the device's language: a title may use any of them.
    val names = listOf(locale.getDisplayLanguage(Locale.ENGLISH), locale.getDisplayLanguage(locale), locale.getDisplayLanguage(display))
        .map { it.lowercase() }
        .filter { it.isNotEmpty() && it != lang.lowercase() }
        .toSet()
    if (words.isEmpty()) {
        val shown = locale.getDisplayLanguage(display)
        return if (shown.isNotEmpty() && !shown.equals(lang, ignoreCase = true)) shown.replaceFirstChar { it.titlecase(display) } else lang.uppercase()
    }
    return if (words.any { it.lowercase() in names }) words.joinToString(" · ") else (words + lang.uppercase()).joinToString(" · ")
}
