package io.github.enrell.lain.ui.screens

import androidx.compose.runtime.MutableState
import androidx.compose.runtime.Stable
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.media3.common.Tracks
import androidx.media3.exoplayer.ExoPlayer
import io.github.enrell.lain.Board
import io.github.enrell.lain.Prefs
import io.github.enrell.lain.domain.skipSeconds
import io.github.enrell.lain.player.VideoEffect
import io.github.enrell.lain.ui.PlayerModel
import io.github.enrell.lain.ui.PlayerState
import java.util.concurrent.atomic.AtomicReference

/**
 * What the player's chrome shows and which of its parts are open, shared by
 * the parts [PlayerScreen] is drawn with. One holder, not some thirty
 * delegated locals in the screen's function: with them all in one function
 * of about 800 lines, the release build's PlayerKt failed to load. It lives
 * as long as the screen does (across episodes), as those locals did.
 */
@Stable
internal class PlayerUi(prefs: Prefs) {
    // What the chrome shows, read from the player.
    var playing by mutableStateOf(false)
    var buffering by mutableStateOf(true)
    var ended by mutableStateOf(false)
    var positionMs by mutableStateOf(0L)
    var durationMs by mutableStateOf(0L)
    var bufferedMs by mutableStateOf(0L)
    var muted by mutableStateOf(false)
    var speed by mutableStateOf(prefs.speed)
    var fill by mutableStateOf(false)
    val subtitleScale = listOf(0.8f, 1f, 1.3f)[prefs.subtitleSize.coerceIn(0, 2)]
    val subtitleBox = prefs.subtitleBox

    // What just changed (the audio track, a speed kept from before), said over
    // the picture by [NoteLine], which clears it when its time is up.
    val note: MutableState<String?> = mutableStateOf(null)
    var textTracks by mutableStateOf(listOf<TextTrack>())
    var textOff by mutableStateOf(false)
    /** The account's subtitle rule (D-280) ran on this source's tracks; it runs once, and never over a pick. */
    var textDecided = false
    var audioTracks by mutableStateOf(listOf<TextTrack>())
    /**
     * The file's audio streams while an HLS session plays (D-280): the session
     * carries one, so Media3 lists only it; these are what a pick chooses from,
     * each pick a new session. Empty for a direct or offline play.
     */
    var streamAudio by mutableStateOf(listOf<StreamAudio>())

    // Which list is open over the picture: 0 none, 1 subtitles, 2 audio.
    var tracksPanel by mutableStateOf(0)
    var showRemaining by mutableStateOf(false)

    // Seconds until the next episode starts by itself, once this one ends.
    var countdown by mutableStateOf<Int?>(null)
    var hint by mutableStateOf(!prefs.playerHintSeen)

    var controls by mutableStateOf(true)
    var settings by mutableStateOf(false)
    var landscape by mutableStateOf(true)

    // Dragging the seek bar: the fraction under the finger, or null.
    var scrub by mutableStateOf<Float?>(null)
    var touched by mutableStateOf(0L)
    var preview by mutableStateOf<ImageBitmap?>(null)

    // The storyboard, once it has arrived: previews are cut out of it.
    var board by mutableStateOf<Board?>(null)

    // Where a seek that needs a new session is headed, until it plays.
    var pendingSec by mutableStateOf<Double?>(null)
    var effect by mutableStateOf(VideoEffect.of(prefs.videoEffect))
    var effectError by mutableStateOf<String?>(null)
    var effectsOn by mutableStateOf(false)
    var skipChoice by mutableStateOf(prefs.skipSec)

    // The size the picture is shown at, read by the effect on the GL thread.
    val shownAt = AtomicReference(0 to 0)

    // The running total of a burst of skips, and its side (-1 back, 1 forward).
    var flash by mutableStateOf<Pair<Int, Int>?>(null)
    var lastSkip by mutableStateOf(0L)

    // Shown in a corner (picture in picture) while something else is on screen.
    var inPip by mutableStateOf(false)
}

/**
 * What the chrome does to the playback: seek, skip, play or pause.
 * Everything the bar shows and takes is in source seconds: the whole
 * title, whatever part of it the current session holds. [state] is the
 * screen's collected state, read when a move is made.
 */
internal class PlayerMoves(
    private val model: PlayerModel,
    private val player: ExoPlayer,
    private val ui: PlayerUi,
    private val state: State<PlayerState>,
) {
    fun totalSec(): Double = model.durationSec(ui.durationMs)
    fun nowSec(): Double = ui.pendingSec ?: model.sourceSec(ui.positionMs)
    fun seekToSec(target: Double) {
        val total = totalSec()
        val t = if (total > 0) target.coerceIn(0.0, total) else target.coerceAtLeast(0.0)
        if (model.needsRebuild(t, player.duration)) {
            // Outside what this session holds: a new one that begins there.
            ui.pendingSec = t
            model.rebuild(t)
            return
        }
        val offset = state.value.source?.offsetSec ?: 0.0
        player.seekTo(((t - offset) * 1000).toLong().coerceAtLeast(0))
        ui.positionMs = player.currentPosition
    }
    fun seekBy(deltaMs: Long) = seekToSec(nowSec() + deltaMs / 1000.0)
    fun skipSec(): Int = skipSeconds(ui.skipChoice, totalSec())
    fun skipMs(): Long = skipSec() * 1000L
    fun skip(side: Int) {
        seekBy(side * skipMs())
        ui.flash = side to ((ui.flash?.takeIf { it.first == side }?.second ?: 0) + skipSec())
        ui.lastSkip = System.currentTimeMillis()
    }
    fun togglePlay() {
        when {
            ui.ended -> { player.seekTo(0); player.play() }
            player.isPlaying -> player.pause()
            else -> player.play()
        }
        ui.touched++
    }
}

internal class TextTrack(val group: Tracks.Group, val index: Int, val label: String, val selected: Boolean)

/** One of the file's audio streams on a transcode: its index, its name, its language tag, whether the session plays it. */
internal class StreamAudio(val stream: Long, val label: String, val language: String, val selected: Boolean)
