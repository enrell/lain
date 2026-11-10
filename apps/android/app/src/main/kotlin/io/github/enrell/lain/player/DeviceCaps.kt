package io.github.enrell.lain.player

import android.media.MediaCodecList
import io.github.enrell.lain.domain.MIME_FAMILIES
import io.github.enrell.lain.domain.decodeClaim

/** The device's decode claim, from its decoders (hardware or software). */
object DeviceCaps {
    val claim: List<String> by lazy {
        val families = mutableSetOf<String>()
        for (info in MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos) {
            if (info.isEncoder) continue
            for (type in info.supportedTypes) {
                MIME_FAMILIES[type.lowercase()]?.let { families += it }
            }
        }
        // AAC decoding is mandatory on Android (CDD), whatever the list says.
        families += "aac"
        decodeClaim(families)
    }
}
