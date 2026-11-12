package io.github.enrell.lain.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.enrell.lain.core.CoreEvent
import io.github.enrell.lain.core.Item
import io.github.enrell.lain.core.LainCoreInterface
import io.github.enrell.lain.core.LainException
import io.github.enrell.lain.core.LibraryInfo
import io.github.enrell.lain.core.ModuleInfo
import io.github.enrell.lain.core.ProgressInfo
import io.github.enrell.lain.core.UserInfo
import io.github.enrell.lain.domain.Title
import io.github.enrell.lain.domain.groupTitles
import io.github.enrell.lain.domain.reading
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

enum class Phase { Starting, Connect, Login, Main }

/** The app's screens; the stack replaces a navigation library. */
/** Not a library: the id of the favourites' page (see [Screen.LibraryDetail]). */
const val FAVORITES = "@favorites"

sealed interface Screen {
    data object Home : Screen
    data object Library : Screen
    data object Read : Screen
    data object Search : Screen
    data object Downloads : Screen
    data object Settings : Screen
    /** A library's page; [FAVORITES] in place of a library's id is the page of the titles kept as favourites. */
    data class LibraryDetail(val id: String) : Screen
    data class TitleDetail(val key: String) : Screen
    data class Player(val itemId: String) : Screen
    data class Reader(val itemId: String) : Screen
}

/**
 * A top-level destination. Ids come from `/api/modules` (D-154, D-250);
 * `downloads` is the app's own.
 */
data class Dest(val id: String, val screen: Screen)

/** Module destinations this app has screens for, in navigation order. */
private val KNOWN: List<Pair<String, Screen>> = listOf(
    "home" to Screen.Home,
    "library" to Screen.Library,
    "search" to Screen.Search,
    "settings" to Screen.Settings,
)

/** When `/api/modules` is not served or unreachable: the core destinations. */
private val FALLBACK = setOf("home", "library", "search", "settings")

fun destinations(modules: List<ModuleInfo>?): List<Dest> {
    val ids = modules?.filter { it.enabled }?.flatMap { m -> m.destinations.map { it.id } }?.toSet() ?: FALLBACK
    val out = KNOWN.filter { it.first in ids }.map { Dest(it.first, it.second) }.toMutableList()
    // Downloads sits before Settings, which stays last.
    val at = out.indexOfFirst { it.id == "settings" }.let { if (it < 0) out.size else it }
    out.add(at, Dest("downloads", Screen.Downloads))
    return out
}

data class AppState(
    val phase: Phase = Phase.Starting,
    val server: String = "",
    val serverVersion: String = "",
    val busy: Boolean = false,
    val error: Throwable? = null,
    /** The server needs its first admin, which the web UI creates. */
    val setupRequired: Boolean = false,
    val user: UserInfo? = null,
    val destinations: List<Dest> = destinations(null),
    /** False while the server is unreachable; offline copies still work. */
    val online: Boolean = true,
    val fromCache: Boolean = false,
    val items: Map<String, Item> = emptyMap(),
    val titles: List<Title> = emptyList(),
    val continues: List<ProgressInfo> = emptyList(),
    val progress: Map<String, ProgressInfo> = emptyMap(),
    val libraries: List<LibraryInfo> = emptyList(),
    /** The account's favourites (its list); null while unknown or when the server has no such list. */
    val favorites: List<io.github.enrell.lain.core.FavoriteInfo>? = null,
    /** How long each video is, in seconds, for the items asked about so far. */
    val durations: Map<String, Double> = emptyMap(),
    /** Metadata overlays by item (genres, synopsis), when the server has them. */
    val overlays: Map<String, io.github.enrell.lain.core.Overlay> = emptyMap(),
    val stack: List<Screen> = listOf(Screen.Home),
    /** The deep place a bar destination was opened from, which Back returns to. */
    val returnTo: List<Screen>? = null,
    /** Where each destination of the bar was left (its own stack), so coming back to it lands there. */
    val tabStacks: Map<Screen, List<Screen>> = emptyMap(),
    val query: String = "",
    val results: List<Item> = emptyList(),
    val searching: Boolean = false,
) {
    val screen: Screen get() = stack.last()
}

/** Session, navigation and the catalog: what every screen shares. */
class AppViewModel(
    private val core: LainCoreInterface,
    private val rememberServer: (String) -> Unit = {},
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {
    private val _state = MutableStateFlow(AppState())
    val state: StateFlow<AppState> = _state.asStateFlow()
    private var events: Job? = null
    private var searchJob: Job? = null

    /** Picks up the last server and its saved session. */
    fun start(lastServer: String?) {
        if (lastServer.isNullOrBlank()) {
            _state.update { it.copy(phase = Phase.Connect) }
            return
        }
        val server = runCatching { core.setServer(lastServer) }.getOrElse {
            _state.update { s -> s.copy(phase = Phase.Connect, error = it) }
            return
        }
        val session = core.session()
        _state.update { it.copy(server = server, phase = if (session != null) Phase.Main else Phase.Login) }
        if (session != null) enterMain()
    }

    fun connect(url: String) {
        val raw = url.trim().let { if ("://" in it) it else "http://$it" }
        _state.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                val server = core.setServer(raw)
                val version = core.health()
                val setup = runCatching { core.setupRequired() }.getOrDefault(false)
                rememberServer(server)
                val session = core.session()
                _state.update {
                    it.copy(
                        busy = false,
                        server = server,
                        serverVersion = version,
                        setupRequired = setup,
                        phase = if (session != null && !setup) Phase.Main else Phase.Login,
                    )
                }
                if (session != null && !setup) enterMain()
            } catch (e: LainException) {
                _state.update { it.copy(busy = false, error = e) }
            }
        }
    }

    fun login(username: String, password: String) {
        if (username.isBlank() || password.isEmpty()) return
        _state.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                val user = core.login(username.trim(), password)
                _state.update { it.copy(busy = false, user = user, phase = Phase.Main, stack = listOf(Screen.Home)) }
                enterMain()
            } catch (e: LainException) {
                _state.update { it.copy(busy = false, error = e) }
            }
        }
    }

    /** Back to the server screen (another server). */
    fun changeServer() {
        events?.cancel()
        _state.update { AppState(phase = Phase.Connect, server = it.server) }
    }

    fun logout() {
        events?.cancel()
        runCatching { core.logout() }
        _state.update { AppState(phase = Phase.Login, server = it.server) }
    }

    fun dismissError() = _state.update { it.copy(error = null) }

    // ---- navigation -----------------------------------------------------

    /**
     * Opens a screen. A destination of the bar replaces the stack, and
     * remembers the place it was opened from when that was somewhere deep
     * (a title, a library), so Back from Settings returns there, not Home.
     */
    fun go(screen: Screen) = _state.update { s ->
        if (s.destinations.none { it.screen == screen }) return@update s.copy(stack = s.stack + screen)
        // Each destination keeps its own stack: leaving Library from a title and coming back
        // lands on that title again; the destination already open goes back to its top.
        val here = s.stack.firstOrNull()
        val kept = s.stack.filterNot { it is Screen.Player || it is Screen.Reader }
        val stacks = if (here != null && kept.isNotEmpty()) s.tabStacks + (here to kept) else s.tabStacks
        if (here == screen) return@update s.copy(stack = listOf(screen), tabStacks = stacks - screen, returnTo = null)
        val from = if (kept.size > 1) kept else s.returnTo
        val clean = if (screen == Screen.Search && s.screen != Screen.Search) s.copy(query = "", results = emptyList(), searching = false) else s
        val target = stacks[screen]?.takeIf { it.isNotEmpty() } ?: listOf(screen)
        // Home is where Back ends: it never leads back into another place.
        clean.copy(stack = target, tabStacks = stacks, returnTo = if (screen != Screen.Home && target.size == 1 && from != null && from.first() != screen) from else null)
    }

    /** Puts [screen] in place of the current one (the next episode takes the player's place). */
    fun replace(screen: Screen) = _state.update { it.copy(stack = it.stack.dropLast(1) + screen) }

    /** Pops a screen; false when the stack is at its root (the system handles back). */
    fun back(): Boolean {
        val s = _state.value
        if (s.stack.size <= 1) {
            s.returnTo?.let { place ->
                _state.update { it.copy(stack = place, returnTo = null) }
                return true
            }
            if (s.screen != Screen.Home && s.phase == Phase.Main) {
                _state.update { it.copy(stack = listOf(Screen.Home)) }
                return true
            }
            return false
        }
        _state.update { it.copy(stack = it.stack.dropLast(1)) }
        if (s.screen is Screen.Player || s.screen is Screen.Reader) refreshProgress()
        return true
    }

    /** Opens an item where it belongs: the reader for books, else the player. */
    fun open(item: Item) = go(if (item.reading()) Screen.Reader(item.id) else Screen.Player(item.id))

    fun title(key: String): Title? = _state.value.titles.firstOrNull { it.key == key }

    // ---- data -------------------------------------------------------------

    private fun enterMain() {
        refresh()
        events?.cancel()
        events = viewModelScope.launch {
            while (isActive) {
                when (core.nextEvent() ?: break) {
                    is CoreEvent.Connected -> {
                        _state.update { it.copy(online = true) }
                        if (_state.value.fromCache) refresh() else refreshProgress()
                    }
                    is CoreEvent.Reconnecting -> _state.update { it.copy(online = false) }
                    is CoreEvent.AuthRequired -> {
                        _state.update { AppState(phase = Phase.Login, server = it.server) }
                        break
                    }
                    is CoreEvent.ProgressChanged -> refreshProgress()
                    is CoreEvent.LibraryChanged -> refresh()
                    is CoreEvent.ModulesChanged -> loadModules()
                }
            }
        }
    }

    private suspend fun loadModules() {
        val modules = try {
            core.modules()
        } catch (e: LainException) {
            null
        }
        _state.update { it.copy(destinations = destinations(modules)) }
    }

    /** Reloads modules, the user, the catalog, libraries and Continue. */
    fun refresh() {
        viewModelScope.launch {
            _state.update { it.copy(busy = true) }
            loadModules()
            runCatching { core.me() }.onSuccess { u -> _state.update { it.copy(user = u) } }
            try {
                val snap = core.catalog()
                val items = snap.items.associateBy { it.id }
                _state.update {
                    it.copy(
                        items = items,
                        titles = groupTitles(snap.items),
                        fromCache = snap.fromCache,
                        online = !snap.fromCache,
                        error = null,
                    )
                }
                if (!snap.fromCache) loadOverlays(snap.items.map { it.id })
            } catch (e: LainException) {
                if (e is LainException.Unauthorized) {
                    _state.update { AppState(phase = Phase.Login, server = it.server) }
                    return@launch
                }
                _state.update { it.copy(error = e, online = !offline(e)) }
            }
            runCatching { core.libraries() }.onSuccess { l -> _state.update { it.copy(libraries = l) } }
            runCatching { withContext(io) { core.favorites() } }.onSuccess { f -> _state.update { it.copy(favorites = f) } }
            refreshProgressNow()
            _state.update { it.copy(busy = false) }
        }
    }

    /**
     * Unlocks every locked private library for this session with the
     * account's PIN or password (D-156), then reads the catalog again.
     * The error, when one refuses, is the first library's.
     */
    suspend fun unlock(secret: String, pin: Boolean): Throwable? {
        val locked = _state.value.libraries.filter { it.private && it.locked }
        for (lib in locked) {
            try {
                withContext(io) { core.unlockLibrary(lib.id, secret, pin) }
            } catch (e: LainException) {
                return e
            }
        }
        refresh()
        return null
    }

    /** Locks every private library again. */
    fun lockAll() {
        viewModelScope.launch {
            // Hidden before the server answers: "locked" must be true the moment it is said.
            val open = _state.value.libraries.filter { it.private && !it.locked }
            val ids = open.map { it.id }.toSet()
            _state.update { st -> st.copy(libraries = st.libraries.map { if (it.id in ids) it.copy(locked = true) else it }, titles = st.titles.filter { it.libraryId !in ids }) }
            for (lib in open) {
                runCatching { withContext(io) { core.lockLibrary(lib.id) } }
            }
            refresh()
        }
    }

    /** Puts a title on the account's list, or takes it off; false when the server refused. */
    suspend fun setFavorite(title: io.github.enrell.lain.domain.Title, on: Boolean): Boolean {
        val id = title.items.firstOrNull()?.id ?: return false
        val got = runCatching { withContext(io) { core.setFavorite(id, on) } }.getOrNull() ?: return false
        _state.update { it.copy(favorites = got) }
        return true
    }

    /** How a title is rated; null on a server without ratings, or when it did not answer. */
    suspend fun rating(title: io.github.enrell.lain.domain.Title): io.github.enrell.lain.core.RatingInfo? {
        val id = title.items.firstOrNull()?.id ?: return null
        return runCatching { withContext(io) { core.rating(id) } }.getOrNull()
    }

    /** Scores a title 1 to 10, or takes the score back with 0; null when the server refused. */
    suspend fun rate(title: io.github.enrell.lain.domain.Title, score: Int): io.github.enrell.lain.core.RatingInfo? {
        val id = title.items.firstOrNull()?.id ?: return null
        return runCatching { withContext(io) { core.rate(id, score.toUInt()) } }.getOrNull()
    }

    /** Asks how long these videos are (a title page's episodes, the rails' cards). */
    fun loadDurations(ids: List<String>) {
        val missing = ids.filter { it !in _state.value.durations }
        if (missing.isEmpty()) return
        viewModelScope.launch {
            val got = runCatching { withContext(io) { core.durations(missing) } }.getOrNull() ?: return@launch
            _state.update { it.copy(durations = it.durations + got.associate { d -> d.itemId to d.durationSec }) }
        }
    }

    /**
     * Marks items watched (or not): a finished record at the end, or one
     * back at the start. The length is what is known of the item, so the
     * mark does not invent one.
     */
    fun markWatched(ids: List<String>, watched: Boolean) {
        viewModelScope.launch {
            for (id in ids) {
                val dur = _state.value.durations[id] ?: _state.value.progress[id]?.durationSec ?: 0.0
                runCatching { withContext(io) { core.saveProgress(id, if (watched) dur else 0.0, dur, watched) } }
            }
            loadProgress(ids)
            refreshProgressNow()
        }
    }

    /** Puts a position back as it was (the undo of Start over). */
    fun restoreProgress(p: io.github.enrell.lain.core.ProgressInfo) {
        viewModelScope.launch {
            runCatching { withContext(io) { core.saveProgress(p.itemId, p.positionSec, p.durationSec, p.completed) } }
            loadProgress(listOf(p.itemId))
            refreshProgressNow()
        }
    }

    private var overlayJob: Job? = null

    /** Genres and synopses for the catalog, in the background: the titles are usable before they arrive. */
    private fun loadOverlays(ids: List<String>) {
        overlayJob?.cancel()
        overlayJob = viewModelScope.launch {
            val got = runCatching { withContext(io) { core.overlays(ids) } }.getOrNull() ?: return@launch
            _state.update { it.copy(overlays = got.associateBy { o -> o.itemId }) }
        }
    }

    fun refreshProgress() {
        viewModelScope.launch { refreshProgressNow() }
    }

    private suspend fun refreshProgressNow() {
        try {
            val list = core.continueWatching()
            _state.update { s -> s.copy(continues = list, progress = s.progress + list.associateBy { it.itemId }) }
            // Finished ones too: a poster, a filter and a title page must count the same episodes.
            runCatching { core.allProgress() }.onSuccess { all ->
                _state.update { s -> s.copy(progress = s.progress + all.associateBy { it.itemId }) }
            }
        } catch (e: LainException) {
            if (offline(e)) _state.update { it.copy(online = false) }
        }
    }

    /** Progress of a title's items (the title page's marks). */
    fun loadProgress(ids: List<String>) {
        viewModelScope.launch {
            for (id in ids) {
                val p = try {
                    core.progress(id)
                } catch (e: LainException) {
                    continue
                }
                _state.update { it.copy(progress = it.progress + (id to p)) }
            }
        }
    }

    fun search(query: String) {
        _state.update { it.copy(query = query) }
        searchJob?.cancel()
        if (query.isBlank()) {
            _state.update { it.copy(results = emptyList(), searching = false) }
            return
        }
        searchJob = viewModelScope.launch {
            _state.update { it.copy(searching = true) }
            kotlinx.coroutines.delay(250)
            val found = try {
                core.search(query, 60)
            } catch (e: LainException) {
                _state.update { it.copy(error = e) }
                emptyList()
            }
            _state.update { it.copy(results = found.filter { !it.missing }, searching = false) }
        }
    }

    /** Queues an offline copy; the caller starts the download service. */
    suspend fun download(itemId: String): Result<Unit> = runCatching { core.offlineAdd(itemId) }.map { }

    suspend fun queuedProgress(): Long = withContext(io) { runCatching { core.queuedProgress() }.getOrDefault(0) }

    override fun onCleared() {
        events?.cancel()
        runCatching { core.stopEvents() }
    }
}

fun offline(e: Throwable): Boolean = e is LainException.Offline || e is LainException.Timeout
