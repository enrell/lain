package io.github.enrell.lain.core

import com.sun.jna.CallbackThreadInitializer
import com.sun.jna.Native

/**
 * Keeps the core's runtime threads attached to the JVM once a UniFFI future
 * continuation has run on them.
 *
 * Rust wakes a waiting call from one of its own threads, and JNA attaches
 * that thread for the continuation callback. When the coroutine resumes
 * there (an unconfined dispatcher, as in Compose UI tests), its next poll
 * is ready and UniFFI calls the continuation again, inside the first
 * callback. JNA's default detaches the thread when the inner callback
 * returns, while the outer one is still running, and ART aborts the
 * process ("attempting to detach while still running code"). With
 * `detach = false` the thread stays attached until it exits.
 *
 * Call it before the first call on a [LainCore].
 */
fun keepCallbackThreadsAttached() {
    Native.setCallbackThreadInitializer(
        uniffiRustFutureContinuationCallbackImpl,
        CallbackThreadInitializer(true, false, "lain-core-callback"),
    )
}
