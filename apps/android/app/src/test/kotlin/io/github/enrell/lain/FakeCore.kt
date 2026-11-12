package io.github.enrell.lain

import io.github.enrell.lain.core.CatalogSnapshot
import io.github.enrell.lain.core.ComicBook
import io.github.enrell.lain.core.CopyState
import io.github.enrell.lain.core.CoreEvent
import io.github.enrell.lain.core.DestinationInfo
import io.github.enrell.lain.core.DownloadObserver
import io.github.enrell.lain.core.DownloadRun
import io.github.enrell.lain.core.FlushSummary
import io.github.enrell.lain.core.Item
import io.github.enrell.lain.core.LainCoreInterface
import io.github.enrell.lain.core.LainException
import io.github.enrell.lain.core.LibraryInfo
import io.github.enrell.lain.core.ModuleInfo
import io.github.enrell.lain.core.OfflineCopy
import io.github.enrell.lain.core.OfflinePolicy
import io.github.enrell.lain.core.OfflineUsage
import io.github.enrell.lain.core.PlaySource
import io.github.enrell.lain.core.ProgressInfo
import io.github.enrell.lain.core.SaveOutcome
import io.github.enrell.lain.core.SessionInfo
import io.github.enrell.lain.core.SourceKind
import io.github.enrell.lain.core.UserInfo
import kotlinx.coroutines.channels.Channel

fun item(id: String, title: String, ep: Long = 0, kind: String = "episode", lib: String = "lib-1", updated: Long = 0, missing: Boolean = false) =
    Item(id = id, libraryId = lib, kind = kind, title = title, season = 0, episode = ep, year = 2023, size = 100, updatedAt = updated, missing = missing, fileName = "$id.mkv")

fun progress(id: String, pos: Double, dur: Double = 1440.0, done: Boolean = false, at: Long = 1) =
    ProgressInfo(itemId = id, positionSec = pos, durationSec = dur, completed = done, updatedAt = at)

fun module(id: String, enabled: Boolean, vararg dests: String) =
    ModuleInfo(id = id, name = id, enabled = enabled, state = if (enabled) "active" else "disabled", destinations = dests.map { DestinationInfo(it, it) })

/** An in-memory lain-client: what the view models see, no native library. */
class FakeCore : LainCoreInterface {
    var serverUrl: String? = null
    var sessionInfo: SessionInfo? = null
    var password = "secret-for-tests"
    var offline = false
    var items = listOf(item("e1", "Frieren", 1, updated = 5), item("e2", "Frieren", 2, updated = 6), item("m1", "Tiny Blade", 1, kind = "manga", lib = "lib-2"))
    var modules = listOf(module("core", true, "home", "library", "settings"), module("search", true, "search"), module("read", false, "read"))
    val saved = mutableListOf<ProgressInfo>()
    val progress = mutableMapOf<String, ProgressInfo>()
    val stopped = mutableListOf<String>()
    var source = PlaySource(SourceKind.DIRECT, "http://s/api/items/e1/stream?token=t", "direct play", 0.0, 0.0, 1440.0, emptyList(), "")
    var burned: PlaySource? = null
    val playCaps = mutableListOf<List<String>>()
    var book = ComicBook("m1", "rtl", List(5) { io.github.enrell.lain.core.PageInfo(it.toLong(), 16, 24, "image/png") }, false)
    val events = Channel<CoreEvent?>(Channel.UNLIMITED)

    private fun reachable() {
        if (offline) throw LainException.Offline("connection refused")
    }

    override suspend fun catalog(): CatalogSnapshot = CatalogSnapshot(items, offline, 1)
    override suspend fun continueWatching(): List<ProgressInfo> {
        reachable()
        return progress.values.filter { !it.completed }.sortedByDescending { it.updatedAt }
    }

    override suspend fun allProgress(): List<ProgressInfo> {
        reachable()
        return progress.values.sortedByDescending { it.updatedAt }
    }
    override suspend fun episodes(itemId: String): List<Item> = items
    override suspend fun flushProgress() = FlushSummary(0, 0, 0, 0, null)
    override suspend fun health(): String {
        reachable()
        return "0.1.0"
    }
    override suspend fun libraries(): List<LibraryInfo> {
        reachable()
        return listOf(LibraryInfo("lib-1", "Anime", "anime", false, false), LibraryInfo("lib-2", "Manga", "manga", false, false))
    }
    override suspend fun login(username: String, password: String): UserInfo {
        reachable()
        if (password != this.password) throw LainException.LoginFailed("invalid credentials")
        sessionInfo = SessionInfo(serverUrl ?: "", "user-$username", username)
        return me()
    }
    override fun logout() {
        sessionInfo = null
    }
    var language = "jpn"
    override suspend fun me(): UserInfo = UserInfo(sessionInfo?.userId ?: "", sessionInfo?.username ?: "", "user", "", language)
    override suspend fun setPreferredLanguage(language: String): UserInfo {
        this.language = language
        return me()
    }
    override suspend fun modules(): List<ModuleInfo> {
        reachable()
        return modules
    }
    override suspend fun nextEvent(): CoreEvent? = events.receive()
    override suspend fun offlineAdd(itemId: String) = copy(itemId, CopyState.QUEUED)
    override fun offlineCopies(): List<OfflineCopy> = emptyList()
    override fun offlineCopy(itemId: String): OfflineCopy? = null
    override suspend fun offlineFetch(itemId: String, observer: DownloadObserver?) = copy(itemId, CopyState.DONE)
    override fun offlinePause(itemId: String) = copy(itemId, CopyState.PAUSED)
    override fun offlinePolicy() = OfflinePolicy(10L shl 30, 5L shl 30, true)
    override fun offlineRemove(itemId: String): Long = 0
    override fun offlineResume(itemId: String) = copy(itemId, CopyState.QUEUED)
    override suspend fun offlineRun(observer: DownloadObserver?) = DownloadRun(0, 0, null)
    override fun offlineUsage() = OfflineUsage(0, 10L shl 30, 5L shl 30, null)
    override suspend fun progress(itemId: String): ProgressInfo = progress[itemId] ?: progress(itemId, 0.0, 0.0, at = 0)
    override fun queuedProgress(): Long = 0
    override suspend fun refreshWatched(): Long = 0
    val favoriteIds = mutableListOf<String>()

    private fun favoriteList() = favoriteIds.mapNotNull { id -> items.firstOrNull { it.id == id } }.map { io.github.enrell.lain.core.FavoriteInfo(it.id, it.kind, it.title) }

    override suspend fun favorites(): List<io.github.enrell.lain.core.FavoriteInfo> = favoriteList()

    override suspend fun setFavorite(itemId: String, on: Boolean): List<io.github.enrell.lain.core.FavoriteInfo> {
        favoriteIds.remove(itemId)
        if (on) favoriteIds.add(itemId)
        return favoriteList()
    }

    val scores = mutableMapOf<String, UInt>()

    private fun ratingOf(itemId: String): io.github.enrell.lain.core.RatingInfo {
        val mine = scores[itemId] ?: 0u
        return io.github.enrell.lain.core.RatingInfo(mine, mine.toDouble(), if (mine > 0u) 1u else 0u)
    }

    override suspend fun rating(itemId: String): io.github.enrell.lain.core.RatingInfo = ratingOf(itemId)

    override suspend fun rate(itemId: String, score: UInt): io.github.enrell.lain.core.RatingInfo {
        if (score == 0u) scores.remove(itemId) else scores[itemId] = score
        return ratingOf(itemId)
    }

    override suspend fun durations(itemIds: List<String>): List<io.github.enrell.lain.core.DurationInfo> = emptyList()

    override suspend fun saveProgress(itemId: String, positionSec: Double, durationSec: Double, completed: Boolean): SaveOutcome {
        val p = progress(itemId, positionSec, durationSec, completed, at = saved.size + 10L)
        saved += p
        progress[itemId] = p
        return if (offline) SaveOutcome.Queued("server unreachable") else SaveOutcome.Synced(p)
    }
    override suspend fun search(query: String, limit: Long): List<Item> = items.filter { it.title.contains(query, ignoreCase = true) }
    override fun server(): String? = serverUrl
    override fun session(): SessionInfo? = sessionInfo
    override fun setOfflinePolicy(policy: OfflinePolicy) = Unit
    override fun setServer(url: String): String {
        if (!url.startsWith("http://")) throw LainException.Unsupported("only http")
        serverUrl = url.trimEnd('/')
        return serverUrl!!
    }
    override suspend fun setupRequired(): Boolean = false
    override fun stopEvents() {
        events.trySend(null)
    }
    override suspend fun thumbnail(itemId: String): ByteArray = ByteArray(0)
    override suspend fun frame(itemId: String, atSec: Double, width: UInt): ByteArray = ByteArray(0)
    var transcodes = mutableListOf<Triple<Double, String, Long?>>()
    override suspend fun playTranscoded(itemId: String, startSec: Double, quality: String, burnStream: Long?): PlaySource {
        transcodes += Triple(startSec, quality, burnStream)
        return burned ?: PlaySource(SourceKind.HLS, "http://s/hls?session=b", "burned", startSec, startSec, null, emptyList(), "sess-b")
    }
    override suspend fun playQualities(): List<io.github.enrell.lain.core.QualityInfo> = listOf(io.github.enrell.lain.core.QualityInfo("720p", 720, 4000))
    override suspend fun storyboard(itemId: String): io.github.enrell.lain.core.StoryboardSheet = throw LainException.NotFound("")
    override suspend fun overlays(itemIds: List<String>): List<io.github.enrell.lain.core.Overlay> = emptyList()
    override suspend fun unlockLibrary(libraryId: String, secret: String, pin: Boolean): LibraryInfo = LibraryInfo(libraryId, "", "", true, false)
    override suspend fun lockLibrary(libraryId: String): LibraryInfo = LibraryInfo(libraryId, "", "", true, true)
    override suspend fun playBurnedIn(itemId: String, stream: Long, startSec: Double): PlaySource =
        burned ?: PlaySource(SourceKind.HLS, "http://s/hls?session=b", "burned", startSec, startSec, null, emptyList(), "sess-b")
    override suspend fun playSource(itemId: String, caps: List<String>): PlaySource {
        playCaps += caps
        val start = progress[itemId]?.takeIf { !it.completed && it.positionSec > 3 }?.positionSec ?: 0.0
        return source.copy(startSec = start)
    }
    override suspend fun stopTranscode(itemId: String, session: String) {
        stopped += session
    }
    override suspend fun comic(itemId: String): ComicBook = book
    override suspend fun comicPage(itemId: String, index: Long): ByteArray = byteArrayOf(index.toByte())

    private fun copy(id: String, state: CopyState) = OfflineCopy(id, "t", "episode", 0, 1, 100, 0, state, "", "", false)
}
