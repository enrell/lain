package io.github.enrell.lain.ui

import io.github.enrell.lain.core.ComicBook
import io.github.enrell.lain.core.LainCoreInterface
import io.github.enrell.lain.core.LainException
import io.github.enrell.lain.core.OfflineCopy
import io.github.enrell.lain.core.OfflinePolicy
import io.github.enrell.lain.core.OfflineUsage
import io.github.enrell.lain.core.PlaySource
import io.github.enrell.lain.core.QualityInfo
import io.github.enrell.lain.core.SaveOutcome
import io.github.enrell.lain.core.SourceKind
import io.github.enrell.lain.core.SubtitleTrack
import io.github.enrell.lain.domain.MIME_FAMILIES
import io.github.enrell.lain.domain.TrackPrefs
import io.github.enrell.lain.domain.audioIn
import io.github.enrell.lain.domain.audioNeedsSession
import io.github.enrell.lain.domain.withoutFamily
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Past this share of the length, a play counts as watched (D-182). */
const val WATCHED_AT = 0.92

// ---- player ---------------------------------------------------------------

data class PlayerState(
    val loading: Boolean = true,
    val source: PlaySource? = null,
    val error: Throwable? = null,
    /** The last save went to the device queue, not the server. */
    val savedLocally: Boolean = false,
    val completed: Boolean = false,
    /** The transcode quality asked for; empty is the server's choice. */
    val quality: String = "",
    /** The embedded subtitle stream burned into the picture, if any. */
    val burnStream: Long? = null,
    /** What the quality menu offers (a transcode only). */
    val qualities: List<QualityInfo> = emptyList(),
    /** The server first chose to send the file itself: "Original" goes back to it. */
    val directFirst: Boolean = false,
)

/** A seek this close to the range a session holds is still a player seek. */
const val SEEK_GRACE_SEC = 3.0

/**
 * The player's logic, apart from ExoPlayer: where the play comes from
 * (D-246), resume, progress saves through the sync engine and the ASS
 * burn-in fallback (D-247). Positions in and out are player
 * milliseconds; an HLS session that starts at the resume point is
 * offset back to source time here.
 *
 * An HLS session carries one audio stream (D-280): the title's audio is
 * asked for as the session starts, and another track is a new session.
 */
class PlayerModel(
    private val core: LainCoreInterface,
    val itemId: String,
    caps: List<String>,
    private val scope: CoroutineScope,
    /** Codec families this device already failed to decode for this item. */
    failedBefore: Set<String> = emptySet(),
    /** Told when a decoder fails on a family, so the next play skips the attempt. */
    private val onDecoderFailed: (family: String) -> Unit = {},
    /** The account's track preferences: which audio a session the app named none for plays. */
    private val trackPrefs: TrackPrefs = TrackPrefs(),
    /**
     * The audio language a transcode should carry ([io.github.enrell.lain.domain.transcodeAudioLanguage]:
     * the title's remembered one), "" for the server's choice; read at each start, so a
     * pick made while the file played directly holds when a session takes over.
     */
    private val audioLanguage: () -> String = { "" },
) {
    /** The decode claim; narrowed when a decoder fails on something it claimed. */
    private var claim = failedBefore.fold(caps) { c, family -> withoutFamily(c, family) }

    private val _state = MutableStateFlow(PlayerState())
    val state: StateFlow<PlayerState> = _state.asStateFlow()

    /** [quality]: asked for from the start (on mobile data); empty plays as the server chooses. */
    fun load(quality: String = "") {
        _state.update { PlayerState(loading = true) }
        scope.launch {
            try {
                val language = audioLanguage()
                val first = core.playSource(itemId, claim, language)
                // The title's audio: the stream the core asked for, else the one this side finds
                // in the title's language (a tag the core did not fold), which then takes a new
                // session, as a quality asked for from the start does.
                val audio = first.audioStream ?: audioIn(first.audio, language)
                val retune = audio != null && audioNeedsSession(first, audio, trackPrefs)
                // A file kept on the device is never converted; otherwise the chosen quality from the same place.
                val src = if ((quality.isNotEmpty() || retune) && first.kind != SourceKind.OFFLINE) {
                    runCatching { core.playTranscoded(itemId, first.startSec, quality, null, audio) }.getOrNull()?.let {
                        if (first.kind == SourceKind.HLS && first.session.isNotEmpty()) runCatching { core.stopTranscode(itemId, first.session) }
                        _state.update { s -> s.copy(quality = quality) }
                        it.copy(durationSec = it.durationSec ?: first.durationSec, subtitles = first.subtitles, audio = first.audio)
                    } ?: first
                } else first
                _state.update { it.copy(loading = false, source = src, directFirst = src.kind == SourceKind.DIRECT) }
                // Offered whatever the first delivery: a file played as it is can still be
                // asked for at a lower quality (a slow network, a data plan).
                if (src.kind != SourceKind.OFFLINE) {
                    runCatching { core.playQualities() }.onSuccess { q -> _state.update { it.copy(qualities = q) } }
                }
            } catch (e: LainException) {
                _state.update { it.copy(loading = false, error = e) }
            }
        }
    }

    /** Where the player should seek once prepared, in ms. */
    fun seekOnStartMs(src: PlaySource): Long = ((src.startSec - src.offsetSec).coerceAtLeast(0.0) * 1000).toLong()

    /** Source seconds for a player position. */
    fun sourceSec(positionMs: Long): Double = (_state.value.source?.offsetSec ?: 0.0) + positionMs.coerceAtLeast(0) / 1000.0

    /** The length in source seconds, from the plan or the player. */
    fun durationSec(playerDurationMs: Long): Double {
        val src = _state.value.source
        src?.durationSec?.let { if (it > 0) return it }
        if (playerDurationMs <= 0) return 0.0
        return (src?.offsetSec ?: 0.0) + playerDurationMs / 1000.0
    }

    /** Saves the position (every few seconds, on pause, at the end). */
    /**
     * Set when the item was already watched as it opened: a second look keeps
     * it watched (a few seconds of a finished episode used to un-mark it and
     * send Resume back to it); only Mark unwatched takes the mark away.
     */
    var rewatch = false

    fun report(positionMs: Long, playerDurationMs: Long, ended: Boolean = false): Job? {
        if (_state.value.source == null) return null
        val pos = sourceSec(positionMs)
        val dur = durationSec(playerDurationMs)
        val done = rewatch || ended || (dur > 0 && pos / dur >= WATCHED_AT)
        if (pos < 1 && !done) return null
        return scope.launch {
            try {
                val out = core.saveProgress(itemId, if (done) dur.coerceAtLeast(pos) else pos, dur, done)
                _state.update { it.copy(savedLocally = out is SaveOutcome.Queued, completed = done) }
            } catch (e: LainException) {
                _state.update { it.copy(error = e) }
            }
        }
    }

    /** Typeset (ASS/SSA) tracks the server can burn in. */
    fun burnable(): List<SubtitleTrack> = _state.value.source?.subtitles?.filter { it.typeset && it.stream >= 0 }.orEmpty()

    /** Switches to an HLS session with [track] burned in, from the current position. */
    fun burnIn(track: SubtitleTrack, positionMs: Long) = rebuild(sourceSec(positionMs), burnStream = track.stream)

    /**
     * A new HLS session whose timeline begins at [fromSec]: how a seek
     * outside the range the current session holds, a quality change and a
     * burn-in all keep the viewer where they were (the web player's
     * `rebuildAt`). The length and the subtitle list carry over.
     */
    /** Back to the delivery the server first chose (the file itself, when the device plays it), from [fromSec]. */
    fun original(fromSec: Double) {
        val previous = _state.value.source
        _state.update { it.copy(loading = true, error = null, quality = "") }
        scope.launch {
            try {
                val src = core.playSource(itemId, claim, audioLanguage())
                stopSession(previous)
                _state.update { it.copy(loading = false, source = src.copy(startSec = fromSec, offsetSec = 0.0)) }
            } catch (e: LainException) {
                _state.update { it.copy(loading = false, error = e) }
            }
        }
    }

    fun rebuild(
        fromSec: Double,
        quality: String = _state.value.quality,
        burnStream: Long? = _state.value.burnStream,
        audioStream: Long? = keptAudio(),
    ) {
        val previous = _state.value.source
        _state.update { it.copy(loading = true, error = null, quality = quality, burnStream = burnStream) }
        scope.launch {
            try {
                val src = core.playTranscoded(itemId, fromSec.coerceAtLeast(0.0), quality, burnStream, audioStream)
                stopSession(previous)
                _state.update {
                    it.copy(
                        loading = false,
                        source = src.copy(
                            durationSec = src.durationSec ?: previous?.durationSec,
                            subtitles = if (burnStream == null) previous?.subtitles.orEmpty() else emptyList(),
                            audio = previous?.audio.orEmpty(),
                        ),
                    )
                }
            } catch (e: LainException) {
                _state.update { it.copy(loading = false, error = e) }
            }
        }
    }

    /**
     * The audio a new session keeps: the stream this one was asked for, else
     * the one in the title's language; null leaves the server's choice. A seek,
     * a quality change and a burn-in must not drop a track the viewer picked.
     */
    private fun keptAudio(): Long? {
        val src = _state.value.source ?: return null
        return src.audioStream ?: audioIn(src.audio, audioLanguage())
    }

    /**
     * The viewer picked the file's audio [stream]. An HLS session carries one,
     * so another stream is a new session from [fromSec]; a direct or offline
     * play switches inside the file (Media3), and the stream that already plays
     * needs nothing. True when a new session was asked for.
     */
    fun switchAudio(stream: Long, fromSec: Double): Boolean {
        val src = _state.value.source ?: return false
        if (!audioNeedsSession(src, stream, trackPrefs)) return false
        rebuild(fromSec, audioStream = stream)
        return true
    }

    /** The source seconds the current session holds: from its start to what the player has of it. */
    fun sessionRange(playerDurationMs: Long): ClosedFloatingPointRange<Double> {
        val offset = _state.value.source?.offsetSec ?: 0.0
        return offset..(offset + playerDurationMs.coerceAtLeast(0) / 1000.0)
    }

    /** Whether a seek to [targetSec] needs a new session instead of a player seek. */
    fun needsRebuild(targetSec: Double, playerDurationMs: Long): Boolean {
        if (_state.value.source?.kind != SourceKind.HLS) return false
        val held = sessionRange(playerDurationMs)
        return targetSec < held.start - SEEK_GRACE_SEC || targetSec > held.endInclusive + SEEK_GRACE_SEC
    }

    /**
     * ExoPlayer could not decode a codec the claim offered (a hardware
     * HEVC decoder with no Main10, say). Ask the server again without that
     * codec family, so it transcodes instead of the player stopping on an
     * error. False when there is nothing to retry.
     */
    fun decoderFailed(mime: String?, positionMs: Long, playerDurationMs: Long): Boolean {
        val src = _state.value.source ?: return false
        val family = mime?.let { MIME_FAMILIES[it] } ?: return false
        if (src.kind == SourceKind.HLS || claim.none { it.endsWith("/$family") }) return false
        claim = withoutFamily(claim, family)
        onDecoderFailed(family)
        _state.update { it.copy(loading = true, error = null) }
        scope.launch {
            report(positionMs, playerDurationMs)?.join()
            try {
                val next = core.playSource(itemId, claim, audioLanguage())
                _state.update { it.copy(loading = false, source = next) }
                if (next.kind == SourceKind.HLS) {
                    runCatching { core.playQualities() }.onSuccess { q -> _state.update { it.copy(qualities = q) } }
                }
            } catch (e: LainException) {
                _state.update { it.copy(loading = false, error = e) }
            }
        }
        return true
    }

    /** A playback error with no fallback: shown in the player. */
    fun playerFailed(e: Throwable) {
        _state.update { it.copy(loading = false, error = e) }
    }

    private suspend fun stopSession(src: PlaySource?) {
        if (src?.kind == SourceKind.HLS && src.session.isNotEmpty()) {
            runCatching { core.stopTranscode(itemId, src.session) }
        }
    }

    /** The player is closing: a last save, then the transcode session stops. */
    fun close(positionMs: Long, playerDurationMs: Long): Job = scope.launch {
        report(positionMs, playerDurationMs)?.join()
        stopSession(_state.value.source)
    }
}

// ---- reader -----------------------------------------------------------------

data class ReaderState(
    val loading: Boolean = true,
    val book: ComicBook? = null,
    /** 0-based. */
    val page: Int = 0,
    val error: Throwable? = null,
) {
    val count: Int get() = book?.pages?.size ?: 0
    val rtl: Boolean get() = book?.direction == "rtl"
}

/**
 * The reader's logic: the page list (from the offline copy when there is
 * one, D-248), resume at the saved page, and saves on every turn
 * (`position_sec` = 1-based page, `duration_sec` = page count).
 */
class ReaderModel(
    private val core: LainCoreInterface,
    val itemId: String,
    private val scope: CoroutineScope,
) {
    private val _state = MutableStateFlow(ReaderState())
    val state: StateFlow<ReaderState> = _state.asStateFlow()
    private val pages = object : LinkedHashMap<Int, ByteArray>(8, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Int, ByteArray>?) = size > 6
    }
    private var save: Job? = null

    fun load() {
        scope.launch {
            try {
                val book = core.comic(itemId)
                val saved = runCatching { core.progress(itemId) }.getOrNull()
                val n = book.pages.size
                val page = when {
                    saved == null || saved.completed -> 0
                    else -> (saved.positionSec.toInt() - 1).coerceIn(0, (n - 1).coerceAtLeast(0))
                }
                _state.update { ReaderState(loading = false, book = book, page = page) }
            } catch (e: LainException) {
                _state.update { ReaderState(loading = false, error = e) }
            }
        }
    }

    /** One page's bytes (a few recent pages are kept). */
    suspend fun page(index: Int): ByteArray? {
        synchronized(pages) { pages[index] }?.let { return it }
        val book = _state.value.book ?: return null
        val info = book.pages.getOrNull(index) ?: return null
        val bytes = try {
            core.comicPage(itemId, info.index)
        } catch (e: LainException) {
            return null
        }
        synchronized(pages) { pages[index] = bytes }
        return bytes
    }

    /** Next page in reading order (the screen maps arrows by direction). */
    fun next() = goTo(_state.value.page + 1)

    fun previous() = goTo(_state.value.page - 1)

    fun goTo(page: Int) {
        val s = _state.value
        if (s.count == 0) return
        val p = page.coerceIn(0, s.count - 1)
        if (p == s.page) return
        _state.update { it.copy(page = p) }
        persist(p)
    }

    private fun persist(page: Int) {
        val n = _state.value.count
        save?.cancel()
        save = scope.launch {
            runCatching { core.saveProgress(itemId, (page + 1).toDouble(), n.toDouble(), page == n - 1) }
        }
    }

    /** The reader is closing: the current page is saved. */
    fun close(): Job? {
        val s = _state.value
        if (s.count == 0) return null
        return scope.launch {
            save?.join()
            runCatching { core.saveProgress(itemId, (s.page + 1).toDouble(), s.count.toDouble(), s.page == s.count - 1) }
        }
    }
}

// ---- downloads ----------------------------------------------------------------

data class DownloadsState(
    val copies: List<OfflineCopy> = emptyList(),
    val usage: OfflineUsage? = null,
    val policy: OfflinePolicy? = null,
    val error: Throwable? = null,
)

class DownloadsModel(
    private val core: LainCoreInterface,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) {
    private val _state = MutableStateFlow(DownloadsState())
    val state: StateFlow<DownloadsState> = _state.asStateFlow()

    fun refresh() {
        scope.launch {
            try {
                val (copies, usage, policy) = withContext(io) {
                    Triple(core.offlineCopies(), core.offlineUsage(), core.offlinePolicy())
                }
                _state.update { DownloadsState(copies = copies, usage = usage, policy = policy) }
            } catch (e: LainException) {
                _state.update { it.copy(error = e) }
            }
        }
    }

    private fun act(f: () -> Unit) {
        scope.launch {
            try {
                withContext(io) { f() }
            } catch (e: LainException) {
                _state.update { it.copy(error = e) }
            }
            refresh()
        }
    }

    fun pause(itemId: String) = act { core.offlinePause(itemId) }

    fun resume(itemId: String) = act { core.offlineResume(itemId) }

    fun remove(itemId: String) = act { core.offlineRemove(itemId) }
}
