package io.github.enrell.lain.ui.screens

/**
 * The orientation outside the player. A phone's screens are laid out
 * upright, so a phone goes back upright by request instead of being left
 * to the system (with rotation locked it could stay on its side after a
 * video); a tablet follows the system.
 *
 * In its own file because MainActivity asks for it as the app starts: kept
 * in Player.kt it made the start load the player's file class, and a
 * release build that failed to load that class stopped the app at launch.
 */
fun restOrientation(activity: android.app.Activity?): Int =
    if ((activity?.resources?.configuration?.smallestScreenWidthDp ?: 0) < 600) android.content.pm.ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
    else android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
