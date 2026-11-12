package io.github.enrell.lain

import io.github.enrell.lain.ui.screens.pageAcrossMiddle
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ReaderTest {
    @Test
    fun scrollModeReadsThePageAcrossTheMiddleNotTheSliverAtTheTop() {
        // Page 1 shows only its last 240 px; page 2 fills the rest of a 1600 px screen.
        val visible = listOf(0 to -790, 1 to 240, 2 to 1270)
        assertEquals(1, pageAcrossMiddle(visible, middle = 800, pages = 8))
    }

    @Test
    fun aPageStartingBelowTheMiddleIsNotBeingReadYet() {
        val visible = listOf(3 to -100, 4 to 930)
        assertEquals(3, pageAcrossMiddle(visible, middle = 800, pages = 8))
    }

    @Test
    fun theEndCardIsNotAPage() {
        // The last page is nearly gone and the end-of-volume card holds the middle.
        val visible = listOf(7 to -900, 8 to 130)
        assertEquals(7, pageAcrossMiddle(visible, middle = 800, pages = 8))
    }

    @Test
    fun noVisiblePageHasNoReading() {
        assertNull(pageAcrossMiddle(emptyList(), middle = 800, pages = 8))
        assertNull(pageAcrossMiddle(listOf(8 to 0), middle = 800, pages = 8))
    }
}
