package io.github.enrell.lain

import android.content.SharedPreferences
import io.github.enrell.lain.core.OfflinePolicy

/** Small personal settings; they apply at once (D-087). */
class Prefs(private val sp: SharedPreferences) {
    var server: String?
        get() = sp.getString("server", null)
        set(v) = sp.edit().putString("server", v).apply()

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
