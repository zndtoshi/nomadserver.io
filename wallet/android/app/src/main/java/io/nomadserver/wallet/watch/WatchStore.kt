package io.nomadserver.wallet.watch

import android.content.Context
import io.nomadserver.wallet.nostr.protocolJson
import kotlinx.serialization.Serializable

/** A watch target the user added: a single address or an extended pubkey. */
@Serializable
data class WatchTarget(
    val kind: Kind,
    val value: String,
    val label: String = "",
) {
    enum class Kind { ADDRESS, XPUB }
}

/** The paired server (not secret; the secret is only ever in RAM on the server). */
@Serializable
data class PairedServer(
    val nodePubkey: String,
    val relays: List<String>,
)

/** Plain-prefs persistence for pairing + watch targets (no secrets here). */
class WatchStore(context: Context) {

    private val prefs = context.getSharedPreferences("nomad_watch", Context.MODE_PRIVATE)

    fun pairedServer(): PairedServer? {
        val raw = prefs.getString(KEY_SERVER, null) ?: return null
        return runCatching {
            protocolJson.decodeFromString(PairedServer.serializer(), raw)
        }.getOrNull()
    }

    fun setPairedServer(server: PairedServer?) {
        if (server == null) {
            prefs.edit().remove(KEY_SERVER).apply()
        } else {
            prefs.edit()
                .putString(KEY_SERVER, protocolJson.encodeToString(PairedServer.serializer(), server))
                .apply()
        }
    }

    fun targets(): List<WatchTarget> {
        val raw = prefs.getString(KEY_TARGETS, null) ?: return emptyList()
        return runCatching {
            protocolJson.decodeFromString(
                kotlinx.serialization.builtins.ListSerializer(WatchTarget.serializer()),
                raw,
            )
        }.getOrDefault(emptyList())
    }

    fun addTarget(target: WatchTarget) {
        val current = targets()
        if (current.any { it.value == target.value }) return
        save(current + target)
    }

    fun removeTarget(value: String) {
        save(targets().filterNot { it.value == value })
    }

    private fun save(list: List<WatchTarget>) {
        prefs.edit()
            .putString(
                KEY_TARGETS,
                protocolJson.encodeToString(
                    kotlinx.serialization.builtins.ListSerializer(WatchTarget.serializer()),
                    list,
                ),
            )
            .apply()
    }

    private companion object {
        const val KEY_SERVER = "paired_server"
        const val KEY_TARGETS = "watch_targets"
    }
}
