package io.github.enrell.lain.ui.theme

import androidx.compose.ui.graphics.Color
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/**
 * The server's palette (`GET /api/theme`, public): the colours the web
 * client paints with, so the app and the site look like one program.
 * Kept as JSON in the preferences so a cold start already has them.
 */
data class WebPalette(
    val background: Color,
    val surface: Color,
    val surfaceHover: Color,
    val surfaceActive: Color,
    val foreground: Color,
    val muted: Color,
    val line: Color,
    val accent: Color,
    val accentForeground: Color,
    val danger: Color,
    val success: Color,
    val warning: Color,
    val dark: Boolean,
) {
    companion object {
        /** The web client's own defaults, used until a server answers. */
        val Default: WebPalette = parse(
            """{"background":"#08090c","surface":"#101218","surface_hover":"#171a22","surface_active":"#1e222c",
               "foreground":"#e7e9f0","muted":"#8a90a3","line":"#1f2430","accent":"#5ce1c4","accent_foreground":"#05231d",
               "danger":"#ff6b81","success":"#6ee7a8","warning":"#ffcf5c","mode":"dark"}""",
        )!!

        fun parse(json: String?): WebPalette? = runCatching {
            val o = JSONObject(json ?: return null)
            fun c(k: String) = Color(android.graphics.Color.parseColor(o.getString(k)))
            WebPalette(
                c("background"), c("surface"), c("surface_hover"), c("surface_active"), c("foreground"), c("muted"),
                c("line"), c("accent"), c("accent_foreground"), c("danger"), c("success"), c("warning"),
                o.optString("mode", "dark") != "light",
            )
        }.getOrNull()

        /** The raw JSON, or null when the server is unreachable or has no theme. */
        suspend fun fetch(server: String): String? = withContext(Dispatchers.IO) {
            runCatching {
                val c = URL(server.trimEnd('/') + "/api/theme").openConnection() as HttpURLConnection
                c.connectTimeout = 4000
                c.readTimeout = 4000
                try {
                    if (c.responseCode == 200) c.inputStream.bufferedReader().readText().takeIf { parse(it) != null } else null
                } finally {
                    c.disconnect()
                }
            }.getOrNull()
        }
    }
}
