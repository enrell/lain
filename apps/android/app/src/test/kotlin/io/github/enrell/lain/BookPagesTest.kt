package io.github.enrell.lain

import io.github.enrell.lain.ui.screens.pageAtFraction
import io.github.enrell.lain.ui.screens.pageCssWidth
import io.github.enrell.lain.ui.screens.pageFraction
import io.github.enrell.lain.ui.screens.pageOfScroll
import io.github.enrell.lain.ui.screens.pageOfText
import io.github.enrell.lain.ui.screens.pageScrollX
import io.github.enrell.lain.ui.screens.pageStride
import io.github.enrell.lain.ui.screens.pagesIn
import io.github.enrell.lain.ui.screens.swipeTurn
import io.github.enrell.lain.ui.screens.tapZone
import org.junit.Assert.assertEquals
import org.junit.Test

class BookPagesTest {
    @Test
    fun thePageWidthIsFlooredToTheLayoutUnit() {
        assertEquals(411.421875, pageCssWidth(1080, 2.625f), 0.0)
        assertEquals(360.0, pageCssWidth(720, 2f), 0.0)
        assertEquals(0.0, pageCssWidth(0, 2f), 0.0)
        assertEquals(0.0, pageCssWidth(1080, 0f), 0.0)
        assertEquals(0.0, pageCssWidth(1080, Float.NaN), 0.0)
        // The stride never passes the view: no page shows a sliver of the next.
        val s = pageStride(1080, 2.625f)
        assertEquals(1079.98, s, 0.01)
    }

    @Test
    fun pagesComeFromTheScrollRange() {
        val s = 1080.0
        assertEquals(1, pagesIn(1080, s))
        assertEquals(1, pagesIn(0, s))
        assertEquals(1, pagesIn(5000, 0.0))
        // The last column's trailing gap may or may not count.
        assertEquals(5, pagesIn(5 * 1080 - 58, s))
        assertEquals(5, pagesIn(5 * 1080, s))
        assertEquals(0, pageOfScroll(0, s, 5))
        assertEquals(2, pageOfScroll(2160, s, 5))
        assertEquals("a scroll near a page start is that page", 2, pageOfScroll(2160 + 58, s, 5))
        assertEquals(4, pageOfScroll(99999, s, 5))
        assertEquals(0, pageOfScroll(-10, s, 5))
        assertEquals(0, pageOfScroll(500, 0.0, 5))
        assertEquals(3240, pageScrollX(3, s))
        // No drift: page 300 of a fractional stride is still rounded once.
        assertEquals(323995, pageScrollX(300, pageStride(1080, 2.625f)))
    }

    @Test
    fun progressIsPageOverLastPage() {
        assertEquals(0.0, pageFraction(0, 5), 0.0)
        assertEquals(0.5, pageFraction(2, 5), 0.0)
        assertEquals(1.0, pageFraction(4, 5), 0.0)
        assertEquals(1.0, pageFraction(0, 1), 0.0)
        assertEquals(1.0, pageFraction(9, 5), 0.0)
        // Resume lands on the page that was saved, whatever the count.
        for (pages in 1..40) for (p in 0 until pages) assertEquals(p, pageAtFraction(pageFraction(p, pages), pages))
        assertEquals(0, pageAtFraction(Double.NaN, 5))
        assertEquals(4, pageAtFraction(2.0, 5))
        assertEquals(0, pageAtFraction(-1.0, 5))
    }

    @Test
    fun anAnchorIsOnThePageHoldingItsText() {
        assertEquals(0, pageOfText(0.0, 4))
        assertEquals(1, pageOfText(0.3, 4))
        assertEquals(3, pageOfText(1.0, 4))
        assertEquals(0, pageOfText(0.7, 1))
        assertEquals(0, pageOfText(Double.NaN, 4))
    }

    @Test
    fun tapsAndSwipes() {
        assertEquals(-1, tapZone(10f, 900))
        assertEquals(0, tapZone(450f, 900))
        assertEquals(1, tapZone(890f, 900))
        assertEquals(0, tapZone(10f, 0))
        assertEquals(1, swipeTurn(-200f, 20f, 1000))
        assertEquals(-1, swipeTurn(200f, -20f, 1000))
        assertEquals(0, swipeTurn(20f, 0f, 1000))
        assertEquals(0, swipeTurn(-200f, 300f, 1000))
        assertEquals(0, swipeTurn(-200f, 0f, 0))
    }
}
