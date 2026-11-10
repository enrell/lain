package io.github.enrell.lain.download

import io.github.enrell.lain.core.DownloadObserver
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/** Live transfer state shared by the download service and the Downloads screen. */
class DownloadTracker : DownloadObserver {
    data class State(
        val running: Boolean = false,
        /** item id -> (bytes done, bytes total). */
        val bytes: Map<String, Pair<Long, Long>> = emptyMap(),
        val evicted: List<String> = emptyList(),
        /** Why the last run stopped early, if it did. */
        val stopped: String? = null,
    )

    private val flow = MutableStateFlow(State())
    val state: StateFlow<State> = flow.asStateFlow()

    fun started() = flow.update { it.copy(running = true, stopped = null) }

    fun finished(stopped: String?) = flow.update { it.copy(running = false, stopped = stopped, bytes = emptyMap()) }

    override fun progress(itemId: String, done: Long, total: Long) {
        flow.update { it.copy(bytes = it.bytes + (itemId to (done to total))) }
    }

    override fun evicted(itemId: String, title: String) {
        flow.update { it.copy(evicted = (it.evicted + title).takeLast(5)) }
    }
}
