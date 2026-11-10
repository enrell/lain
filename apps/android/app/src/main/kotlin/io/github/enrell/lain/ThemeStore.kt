package io.github.enrell.lain

import io.github.enrell.lain.core.ThemeTokens
import io.github.enrell.lain.core.builtinThemes
import io.github.enrell.lain.core.parseTheme
import java.io.File

/**
 * `lain.ui.theme@1` folders (docs/THEMES.md, D-249): `<files>/themes/<id>/theme.json`.
 * The two shipped themes are written there on first run so they can be edited
 * (`adb shell run-as`); the parser is the Rust one, shared with the desktop's rules.
 */
class ThemeStore(private val dir: File) {
    data class Entry(val id: String, val name: String, val scheme: String, val problem: String?)

    fun installBuiltins() {
        for (t in builtinThemes()) {
            val f = File(dir, "${t.id}/theme.json")
            if (!f.exists()) {
                f.parentFile?.mkdirs()
                f.writeText(t.json)
            }
        }
    }

    fun list(): List<Entry> =
        (dir.listFiles() ?: emptyArray()).filter { File(it, "theme.json").isFile }.sortedBy { it.name }.map { folder ->
            runCatching { parseTheme(File(folder, "theme.json").readText()) }.fold(
                onSuccess = { Entry(folder.name, it.name.ifBlank { folder.name }, it.scheme, it.problems.firstOrNull()) },
                onFailure = { Entry(folder.name, folder.name, "dark", it.message ?: it.toString()) },
            )
        }

    /** The theme [id], else the shipped dark one. */
    fun load(id: String): ThemeTokens {
        val file = File(dir, "$id/theme.json")
        val parsed = runCatching { parseTheme(file.readText()) }.getOrNull()
        return parsed ?: parseTheme(builtinThemes().first().json)
    }
}
