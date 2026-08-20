package io.nomadserver.wallet.watch

import android.content.Context
import io.nomadserver.wallet.nostr.IdentityStore
import io.nomadserver.wallet.nostr.NomadClient
import io.nomadserver.wallet.nostr.NotifyEnvelope
import io.nomadserver.wallet.nostr.protocolJson
import io.nomadserver.wallet.pairing.Pairing
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.mapNotNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import rust.nostr.sdk.PublicKey

data class BalanceSnapshot(
    val confirmedSats: Long,
    val unconfirmedSats: Long,
    val history: List<HistoryEntry>,
    val watching: Int,
    val syncedAt: Long,
)

data class HistoryEntry(
    val txid: String,
    val height: Long,
)

/**
 * Orchestrates identity, pairing, watch targets and chain sync. All chain
 * data flows through the Nomad protocol — this app has no direct chain
 * access and holds no Bitcoin keys (watch-only milestone).
 */
class WalletRepository(context: Context) {

    private val appContext = context.applicationContext
    private val store = WatchStore(appContext)
    private val keys = IdentityStore(appContext).getOrCreate()

    private var client: NomadClient? = null

    val walletPubkey: String = keys.publicKey().toHex()

    fun pairedServer(): PairedServer? = store.pairedServer()

    fun targets(): List<WatchTarget> = store.targets()

    /** Notifications from the paired server (notify envelopes only). */
    suspend fun notifications(): Flow<JsonObject>? {
        val server = store.pairedServer() ?: return null
        val serverPk = PublicKey.parse(server.nodePubkey)
        return ensureClient(server).gifts.mapNotNull { gift ->
            if (gift.sender() != serverPk) return@mapNotNull null
            val env = runCatching {
                protocolJson.decodeFromString(NotifyEnvelope.serializer(), gift.rumor().content())
            }.getOrNull()
            env?.takeIf { it.type == "notify" }?.payload
        }
    }

    private suspend fun ensureClient(server: PairedServer): NomadClient {
        client?.let { return it }
        val c = NomadClient(keys)
        c.connect(server.relays)
        client = c
        return c
    }

    /** Full pairing flow: validate payload → connect → pair → persist. */
    suspend fun pair(rawPayload: String) {
        val payload = Pairing.parsePayload(rawPayload, NomadClient.nowSecs())
        val server = PairedServer(payload.nodePubkey, payload.relays)
        val client = ensureClient(server)
        val proof = Pairing.proof(payload.pairSecret, walletPubkey)
        client.request(
            PublicKey.parse(payload.nodePubkey),
            "pair",
            kotlinx.serialization.json.buildJsonObject {
                put("proof", kotlinx.serialization.json.JsonPrimitive(proof))
                put("client", kotlinx.serialization.json.JsonPrimitive("nomad-wallet-android/0.1.0"))
            },
        )
        store.setPairedServer(server)
    }

    suspend fun unpair() {
        val server = store.pairedServer() ?: return
        runCatching {
            ensureClient(server).request(
                PublicKey.parse(server.nodePubkey),
                "unpair",
                JsonObject(emptyMap()),
            )
        }
        store.setPairedServer(null)
        client = null
    }

    /** Add a watch target: single address, or extended pubkey. */
    fun addTarget(input: String): WatchTarget {
        val trimmed = input.trim()
        val target = when {
            Deriver.isExtendedPubkey(trimmed) -> {
                // validate eagerly: derivation must succeed
                Deriver.deriveAddresses(trimmed, mainnet = true)
                WatchTarget(WatchTarget.Kind.XPUB, trimmed)
            }
            isPlausibleAddress(trimmed) -> WatchTarget(WatchTarget.Kind.ADDRESS, trimmed)
            else -> throw Deriver.Invalid("not an address or extended pubkey")
        }
        store.addTarget(target)
        return target
    }

    fun removeTarget(value: String) = store.removeTarget(value)

    /** All watched addresses, with xpub targets expanded. */
    fun allAddresses(): List<String> {
        val out = LinkedHashSet<String>()
        for (t in store.targets()) {
            when (t.kind) {
                WatchTarget.Kind.ADDRESS -> out.add(t.value)
                WatchTarget.Kind.XPUB -> out.addAll(Deriver.deriveAddresses(t.value, mainnet = true))
            }
        }
        return out.toList()
    }

    /** Sync balances + history for all watched addresses, and re-assert
     *  the server-side watch set (replace semantics, PROTOCOL.md §5.8). */
    suspend fun sync(): BalanceSnapshot {
        val server = store.pairedServer()
            ?: throw IllegalStateException("not paired")
        val client = ensureClient(server)
        val serverPk = PublicKey.parse(server.nodePubkey)
        val addresses = allAddresses()

        val watching = if (addresses.isEmpty()) {
            0
        } else {
            val arr = kotlinx.serialization.json.buildJsonArray {
                addresses.forEach { add(kotlinx.serialization.json.JsonPrimitive(it)) }
            }
            val res = client.request(
                serverPk,
                "watch_addresses",
                kotlinx.serialization.json.buildJsonObject { put("addresses", arr) },
            )
            res["watching"]?.jsonPrimitive?.int ?: 0
        }

        if (addresses.isEmpty()) {
            return BalanceSnapshot(0, 0, emptyList(), 0, NomadClient.nowSecs())
        }

        val arr = kotlinx.serialization.json.buildJsonArray {
            addresses.forEach { add(kotlinx.serialization.json.JsonPrimitive(it)) }
        }
        val balance = client.request(
            serverPk,
            "get_balance",
            kotlinx.serialization.json.buildJsonObject { put("addresses", arr) },
        )
        val history = client.request(
            serverPk,
            "get_history",
            kotlinx.serialization.json.buildJsonObject {
                put("addresses", arr)
                put("limit", kotlinx.serialization.json.JsonPrimitive(20))
            },
        )
        val entries = history["txs"]?.let { it as? kotlinx.serialization.json.JsonArray }
            ?.mapNotNull { el ->
                val o = el as? JsonObject ?: return@mapNotNull null
                val txid = o["txid"]?.jsonPrimitive?.content ?: return@mapNotNull null
                val height = o["height"]?.jsonPrimitive?.long ?: 0
                HistoryEntry(txid, height)
            }
            .orEmpty()

        return BalanceSnapshot(
            confirmedSats = balance["confirmed"]?.jsonPrimitive?.long ?: 0,
            unconfirmedSats = balance["unconfirmed"]?.jsonPrimitive?.long ?: 0,
            history = entries,
            watching = watching,
            syncedAt = NomadClient.nowSecs(),
        )
    }

    private fun isPlausibleAddress(s: String): Boolean =
        s.length in 26..90 &&
            (s.startsWith("bc1") || s.startsWith("tb1") || s.startsWith("bcrt1") ||
                s.startsWith("1") || s.startsWith("3") || s.startsWith("m") || s.startsWith("n") ||
                s.startsWith("2"))
}
