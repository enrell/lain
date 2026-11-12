package io.github.enrell.lain.domain

/**
 * The decode claim sent with `/api/items/{id}/playback` (D-058, D-246):
 * container tokens and container/codec pairs from the server's
 * vocabulary (`lain-watch` plan.rs). ExoPlayer demuxes every container
 * listed here itself; codecs come from the device's decoders.
 */
private val CONTAINER_CODECS: Map<String, Set<String>> = mapOf(
    "mp4" to setOf("h264", "hevc", "av1", "vp9", "aac", "mp3", "ac3", "eac3", "opus", "flac"),
    "mkv" to setOf("h264", "hevc", "av1", "vp8", "vp9", "aac", "mp3", "ac3", "eac3", "opus", "vorbis", "flac", "dts"),
    "webm" to setOf("vp8", "vp9", "av1", "opus", "vorbis"),
    "ogg" to setOf("opus", "vorbis", "flac"),
    "mp3" to setOf("mp3"),
    "flac" to setOf("flac"),
)

/** Android MIME types of decoders, mapped to the server's codec families. */
val MIME_FAMILIES: Map<String, String> = mapOf(
    "video/avc" to "h264",
    "video/hevc" to "hevc",
    "video/av01" to "av1",
    "video/x-vnd.on2.vp8" to "vp8",
    "video/x-vnd.on2.vp9" to "vp9",
    "audio/mp4a-latm" to "aac",
    "audio/mpeg" to "mp3",
    "audio/ac3" to "ac3",
    "audio/eac3" to "eac3",
    "audio/opus" to "opus",
    "audio/vorbis" to "vorbis",
    "audio/flac" to "flac",
    "audio/vnd.dts" to "dts",
)

/**
 * [claim] without the codec [family], for when the device's decoder turns
 * out not to handle it (an HEVC decoder with no Main10 profile, say); a
 * container left with no codec drops too.
 */
fun withoutFamily(claim: List<String>, family: String): List<String> {
    val pairs = claim.filter { '/' in it && it.substringAfter('/') != family }
    return (pairs.map { it.substringBefore('/') }.toSet() + pairs).sorted()
}

/**
 * How far one skip goes (the double tap, the buttons, the arrow keys): the
 * viewer's choice, or by the title's length when left to the player. Ten
 * seconds is a step in an episode and nothing in a two hour film.
 */
fun skipSeconds(choice: Int, durationSec: Double): Int = when {
    choice > 0 -> choice
    durationSec >= 3600 -> 30
    else -> 10
}

/** The choices offered; 0 is "by length". */
val SKIP_CHOICES = listOf(0, 5, 10, 30, 60)

/** The claim for a device that decodes [families]; sorted, without duplicates. */
fun decodeClaim(families: Set<String>): List<String> {
    val out = sortedSetOf<String>()
    for ((container, codecs) in CONTAINER_CODECS) {
        val usable = codecs.intersect(families)
        if (usable.isEmpty()) continue
        out += container
        usable.forEach { out += "$container/$it" }
    }
    return out.toList()
}
