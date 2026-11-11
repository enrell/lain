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
