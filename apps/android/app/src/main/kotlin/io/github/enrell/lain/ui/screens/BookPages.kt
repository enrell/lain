package io.github.enrell.lain.ui.screens

import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.roundToInt

/*
 * Page arithmetic of the book reader's paged layout (no Android types,
 * so it is tested on the JVM). A chapter is laid out in columns one page
 * wide (`bookPagedCss`); page k starts k strides to the right, a stride
 * being the page's width in CSS pixels times the display density.
 */

/**
 * The page width in CSS pixels of a view [px] physical pixels wide:
 * floored to the 1/64 px the layout engine counts in, the same rule the
 * stylesheet applies, so the stride scrolled by and the one laid out agree.
 */
internal fun pageCssWidth(px: Int, density: Float): Double {
    if (px <= 0 || !(density > 0f)) return 0.0
    return floor(px / density.toDouble() * 64.0) / 64.0
}

/** One page's stride in view pixels. */
internal fun pageStride(px: Int, density: Float): Double = pageCssWidth(px, density) * density

/** Pages of a chapter whose horizontal scroll range is [range] view pixels (at least one). */
internal fun pagesIn(range: Int, stride: Double): Int =
    if (!(stride > 0.0) || range <= 0) 1 else (range / stride).roundToInt().coerceAtLeast(1)

/** The page a horizontal scroll offset shows: the nearest page start. */
internal fun pageOfScroll(scrollX: Int, stride: Double, pages: Int): Int =
    if (!(stride > 0.0)) 0 else (scrollX / stride).roundToInt().coerceIn(0, (pages - 1).coerceAtLeast(0))

/** Where [page] starts, in view pixels. */
internal fun pageScrollX(page: Int, stride: Double): Int = if (!(stride > 0.0)) 0 else (page.coerceAtLeast(0) * stride).roundToInt()

/** How far into the chapter a page is: page / (pages - 1); a chapter of one page is read once shown. */
internal fun pageFraction(page: Int, pages: Int): Double =
    if (pages <= 1) 1.0 else (page.toDouble() / (pages - 1)).coerceIn(0.0, 1.0)

/** The page a saved fraction lands on: the inverse of [pageFraction]. */
internal fun pageAtFraction(fraction: Double, pages: Int): Int =
    if (pages <= 1 || fraction.isNaN()) 0 else (fraction.coerceIn(0.0, 1.0) * (pages - 1)).roundToInt()

/** The page holding the text [fraction] of the way into the chapter (an anchor's place). */
internal fun pageOfText(fraction: Double, pages: Int): Int =
    if (pages <= 1 || fraction.isNaN()) 0 else floor(fraction.coerceIn(0.0, 1.0) * pages).toInt().coerceIn(0, pages - 1)

/** A tap at [x] on a view [width] wide: -1 on the left third, 1 on the right third, 0 in the middle. */
internal fun tapZone(x: Float, width: Int): Int = when {
    width <= 0 -> 0
    x < width / 3f -> -1
    x > width * 2f / 3f -> 1
    else -> 0
}

/**
 * A drag of [dx], [dy] on a view [width] wide: 1 a swipe to the left
 * (the page after, in a left-to-right book), -1 a swipe to the right,
 * 0 not a horizontal swipe.
 */
internal fun swipeTurn(dx: Float, dy: Float, width: Int): Int {
    if (width <= 0 || abs(dx) <= abs(dy) || abs(dx) < width * 0.06f) return 0
    return if (dx < 0) 1 else -1
}
