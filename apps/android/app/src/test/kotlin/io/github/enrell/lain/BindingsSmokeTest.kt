package io.github.enrell.lain

import io.github.enrell.lain.core.LainCore
import io.github.enrell.lain.core.LainException
import io.github.enrell.lain.core.builtinThemes
import io.github.enrell.lain.core.coreVersion
import io.github.enrell.lain.core.parseTheme
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import java.nio.file.Files

/**
 * The generated bindings against the real Rust library built for the
 * host (CI passes `-PlainHostLib=<path to liblain_ffi.so>`): records,
 * errors and suspend calls cross JNA as the app uses them. Skipped
 * without the library.
 */
class BindingsSmokeTest {
    @Before
    fun needsHostLibrary() {
        val lib = System.getProperty("uniffi.component.lain_ffi.libraryOverride")
        assumeTrue("no host lain-ffi library given", lib != null && java.io.File(lib).isFile)
    }

    @Test
    fun themesParseThroughTheBindings() {
        assertTrue(coreVersion().isNotEmpty())
        val all = builtinThemes()
        assertEquals(listOf("lain-dark", "paper-light"), all.map { it.id })
        val dark = parseTheme(all[0].json)
        assertEquals("dark", dark.scheme)
        assertTrue(dark.problems.isEmpty())
        assertEquals(0xFF, (dark.colors.accent shr 24).toInt() and 0xFF)
        try {
            parseTheme("{")
            throw AssertionError("bad JSON must be refused")
        } catch (e: LainException.Invalid) {
            assertTrue(e.detail.contains("theme.json"))
        }
    }

    @Test
    fun anUnreachableServerIsATypedOfflineError() = runBlocking {
        val dir = Files.createTempDirectory("lain-core").toFile()
        val core = LainCore(dir.resolve("state").path, dir.resolve("copies").path)
        try {
            assertNull(core.session())
            try {
                core.catalog()
                throw AssertionError("no server chosen")
            } catch (e: LainException.NoServer) {
                // expected
            }
            // Port 9 (discard) on loopback: refused at once.
            assertEquals("http://127.0.0.1:9", core.setServer("http://127.0.0.1:9"))
            try {
                core.health()
                throw AssertionError("the server is not there")
            } catch (e: LainException) {
                assertTrue("$e", e is LainException.Offline || e is LainException.Timeout)
            }
            assertTrue(core.offlineCopies().isEmpty())
            assertEquals(0L, core.queuedProgress())
        } finally {
            core.close()
            dir.deleteRecursively()
        }
    }
}
