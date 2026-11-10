package io.github.enrell.lain.ui

import io.github.enrell.lain.core.ComicBook
import io.github.enrell.lain.core.LainCoreInterface
import io.github.enrell.lain.core.LainException
import io.github.enrell.lain.core.OfflineCopy
import io.github.enrell.lain.core.OfflinePolicy
import io.github.enrell.lain.core.OfflineUsage
import io.github.enrell.lain.core.PlaySource
import io.github.enrell.lain.core.SaveOutcome
import io.github.enrell.lain.core.SourceKind
import io.github.enrell.lain.core.SubtitleTrack
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
)

/**
 * The player's logic, apart from ExoPlayer: where the play comes from
 * (D-246), resume, progress saves through the sync engine and the ASS
 * burn-in fallback (D-247). Positions in and out are player
 * milliseconds; an HLS session that starts at the resume point is
 * offset back to source time here.
 */
class PlayerModel(
    private val core: LainCoreInterface,
    val itemId: String,
    private val caps: List<String>,
    private val scope: CoroutineScope,
) {
    private val _state = MutableStateFlow(PlayerState())
    val state: StateFlow<PlayerState> = _state.asStateFlow()

    fun load() {
        _state.update { PlayerState(loading = true) }
        scope.launch {
            try {
                val src = core.playSource(itemId, caps)
                _state.update { it.copy(loading = false, source = src) }
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
    fun report(positionMs: Long, playerDurationMs: Long, ended: Boolean = false): Job? {
        if (_state.value.source == null) return null
        val pos = sourceSec(positionMs)
        val dur = durationSec(playerDurationMs)
        val done = ended || (dur > 0 && pos / dur >= WATCHED_AT)
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
    fun burnIn(track: SubtitleTrack, positionMs: Long) {
        val from = sourceSec(positionMs)
        val previous = _state.value.source
        _state.update { it.copy(loading = true, error = null) }
        scope.launch {
            try {
                val src = core.playBurnedIn(itemId, track.stream, from)
                stopSession(previous)
                _state.update { it.copy(loading = false, source = src.copy(durationSec = src.durationSec ?: previous?.durationSec)) }
            } catch (e: LainException) {
                _state.update { it.copy(loading = false, error = e) }
            }
        }
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
