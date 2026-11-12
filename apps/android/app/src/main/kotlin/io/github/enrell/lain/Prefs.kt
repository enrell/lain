package io.github.enrell.lain

import android.content.SharedPreferences
import io.github.enrell.lain.core.OfflinePolicy

/** Small personal settings; they apply at once (D-087). */
class Prefs(private val sp: SharedPreferences) {
    var server: String?
        get() = sp.getString("server", null)
        set(v) = sp.edit().putString("server", v).apply()

    /** The server palette last fetched (`/api/theme`), as JSON. */
    var palette: String?
        get() = sp.getString("palette", null)
        set(v) = sp.edit().putString("palette", v).apply()

    /** Library page conveniences: the sort used last and the tile size. */
    var librarySort: String
        get() = sp.getString("library_sort", "title") ?: "title"
        set(v) = sp.edit().putString("library_sort", v).apply()

    /** The order one library was last put in ("" is the whole catalog): "most episodes" means nothing among films. */
    fun sortOf(scope: String): String = sp.getString("library_sort:$scope", "title") ?: "title"
    fun setSortOf(scope: String, v: String) = sp.edit().putString("library_sort:$scope", v).apply()

    var smallTiles: Boolean
        get() = sp.getBoolean("small_tiles", false)
        set(v) = sp.edit().putBoolean("small_tiles", v).apply()

    var libraryShelves: Boolean
        get() = sp.getBoolean("library_shelves", true)
        set(v) = sp.edit().putBoolean("library_shelves", v).apply()

    /** The video effect used last in the player (a preset id). */
    var videoEffect: String
        get() = sp.getString("video_effect", "off") ?: "off"
        set(v) = sp.edit().putString("video_effect", v).apply()

    /** How far a skip goes, in seconds; 0 leaves it to the title's length. */
    var skipSec: Int
        get() = sp.getInt("skip_sec", 0)
        set(v) = sp.edit().putInt("skip_sec", v).apply()

    /**
     * Items whose codec the device's decoder failed on, as `item=family`
     * (the newest few hundred): their next play asks for a transcode at
     * once instead of failing first.
     */
    fun failedFamilies(itemId: String): Set<String> =
        (sp.getStringSet("decode_failed", emptySet()) ?: emptySet()).filter { it.startsWith("$itemId=") }.map { it.substringAfter('=') }.toSet()

    fun rememberFailed(itemId: String, family: String) {
        val all = (sp.getStringSet("decode_failed", emptySet()) ?: emptySet()).toMutableList()
        all.remove("$itemId=$family")
        all.add("$itemId=$family")
        sp.edit().putStringSet("decode_failed", all.takeLast(400).toSet()).apply()
    }

    /** The last few searches that found something, newest first. */
    var recentSearches: List<String>
        get() = (sp.getString("recent_searches", "") ?: "").split('\n').filter { it.isNotBlank() }
        set(v) = sp.edit().putString("recent_searches", v.joinToString("\n")).apply()

    /** The question about download notifications was put once. */
    var notifyAsked: Boolean
        get() = sp.getBoolean("notify_asked", false)
        set(v) = sp.edit().putBoolean("notify_asked", v).apply()

    /** The playback speed last chosen, kept from one video to the next. */
    var speed: Float
        get() = sp.getFloat("player_speed", 1f)
        set(v) = sp.edit().putFloat("player_speed", v).apply()

    /** The player's gestures were explained once. */
    var playerHintSeen: Boolean
        get() = sp.getBoolean("player_hint_seen", false)
        set(v) = sp.edit().putBoolean("player_hint_seen", v).apply()

    /** The reader shows pages one under the other (scroll mode) instead of turning them. */
    var readerScroll: Boolean
        get() = sp.getBoolean("reader_scroll", false)
        set(v) = sp.edit().putBoolean("reader_scroll", v).apply()

    /** The reader's gestures were explained once. */
    var readerHintSeen: Boolean
        get() = sp.getBoolean("reader_hint_seen", false)
        set(v) = sp.edit().putBoolean("reader_hint_seen", v).apply()

    var theme: String
        get() = sp.getString("theme", "lain-dark") ?: "lain-dark"
        set(v) = sp.edit().putString("theme", v).apply()

    fun offlinePolicy(): OfflinePolicy? {
        if (!sp.contains("offline_max")) return null
        return OfflinePolicy(
            maxBytes = sp.getLong("offline_max", 0),
            minFreeBytes = sp.getLong("offline_floor", 0),
            evictWatched = sp.getBoolean("offline_evict", true),
        )
    }

    fun setOfflinePolicy(p: OfflinePolicy) {
        sp.edit()
            .putLong("offline_max", p.maxBytes)
            .putLong("offline_floor", p.minFreeBytes)
            .putBoolean("offline_evict", p.evictWatched)
            .apply()
    }
}
