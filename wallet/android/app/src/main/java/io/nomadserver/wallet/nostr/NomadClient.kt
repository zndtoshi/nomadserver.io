package io.nomadserver.wallet.nostr

import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.mapNotNull
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.JsonObject
import rust.nostr.sdk.Client
import rust.nostr.sdk.Event
import rust.nostr.sdk.EventBuilder
import rust.nostr.sdk.Filter
import rust.nostr.sdk.HandleNotification
import rust.nostr.sdk.Keys
import rust.nostr.sdk.Kind
import rust.nostr.sdk.KindStandard
import rust.nostr.sdk.NostrSigner
import rust.nostr.sdk.PublicKey
import rust.nostr.sdk.RelayMessage
import rust.nostr.sdk.RelayUrl
import rust.nostr.sdk.Timestamp
import rust.nostr.sdk.UnsignedEvent
import rust.nostr.sdk.UnwrappedGift

/**
 * Wallet-side protocol client (PROTOCOL.md). One instance per wallet
 * identity. Patterns proven during server development:
 *
 * - one persistent notification stream (never drop/recreate between
 *   attempts — events arriving in the gap are lost)
 * - backfill subscription (since now-3d): gift wraps are stored;
 *   live-only subscriptions miss messages during connection gaps
 * - idempotent retry with the SAME envelope id (server dedupes)
 * - response correlation strictly by envelope id
 */
class NomadClient(val keys: Keys) {

    private val signer: NostrSigner = NostrSigner.keys(keys)
    private val client: Client = Client(signer)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /** Unwrapped gifts from any sender, as they arrive (spam-filtered by decryptability only). */
    private val _gifts = MutableSharedFlow<UnwrappedGift>(
        extraBufferCapacity = 64,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    val gifts: SharedFlow<UnwrappedGift> = _gifts

    @Volatile
    private var notificationsStarted = false

    suspend fun connect(relays: List<String>) {
        for (r in relays) {
            runCatching { client.addRelay(RelayUrl.parse(r)) }
        }
        client.connect()

        val since = Timestamp.fromSecs((nowSecs() - BACKFILL_SECS).toULong())
        val filter = Filter()
            .kind(Kind.fromStd(KindStandard.GIFT_WRAP))
            .pubkey(keys.publicKey())
            .since(since)
        client.subscribe(filter, null)

        if (!notificationsStarted) {
            notificationsStarted = true
            scope.launch {
                client.handleNotifications(object : HandleNotification {
                    override suspend fun handle(relayUrl: RelayUrl, subscriptionId: String, event: Event) {
                        if (event.kind() != Kind.fromStd(KindStandard.GIFT_WRAP)) return
                        // Not decryptable = spam wrap; discard silently.
                        val gift = runCatching { UnwrappedGift.fromGiftWrap(signer, event) }
                            .getOrNull() ?: return
                        _gifts.tryEmit(gift)
                    }

                    override suspend fun handleMsg(relayUrl: RelayUrl, message: RelayMessage) {
                        // OK/CLOSED/NOTICE: ignored at this layer
                    }
                })
            }
        }
    }

    /**
     * Send a request to [serverPk] and await its response, resending with
     * the same envelope id on timeout (all messages are idempotent).
     * Returns the response `result` payload; throws [ProtocolException] on
     * a protocol error, [java.io.IOException] on transport failure.
     */
    suspend fun request(
        serverPk: PublicKey,
        type: String,
        payload: JsonObject,
        attempts: Int = 3,
    ): JsonObject {
        val id = UUID.randomUUID().toString()
        val envelope = RequestEnvelope(
            id = id,
            ts = nowSecs(),
            type = type,
            payload = payload,
        )
        val rumor: UnsignedEvent = EventBuilder(
            Kind(KIND_REQUEST.toUShort()),
            protocolJson.encodeToString(RequestEnvelope.serializer(), envelope),
        ).build(keys.publicKey())

        var lastError: Exception = java.io.IOException("no attempts")
        repeat(attempts) {
            client.giftWrap(serverPk, rumor, emptyList())
            val response = withTimeoutOrNull(ATTEMPT_TIMEOUT_MS) {
                gifts.mapNotNull { gift -> parseResponse(gift, serverPk, id) }.first()
            }
            if (response != null) return response
            lastError = java.io.IOException("timeout waiting for $type response")
        }
        throw lastError
    }

    private fun parseResponse(gift: UnwrappedGift, serverPk: PublicKey, id: String): JsonObject? {
        if (gift.sender() != serverPk) return null
        if (gift.rumor().kind() != Kind(KIND_RESPONSE.toUShort())) return null
        val env = runCatching {
            protocolJson.decodeFromString(ResponseEnvelope.serializer(), gift.rumor().content())
        }.getOrNull() ?: return null
        if (env.id != id) return null
        if (env.ok) return env.result ?: JsonObject(emptyMap())
        val err = env.error
        throw ProtocolException(err?.code ?: "internal", err?.message ?: "unknown error")
    }

    companion object {
        private const val ATTEMPT_TIMEOUT_MS = 25_000L
        fun nowSecs(): Long = System.currentTimeMillis() / 1000
    }
}

class ProtocolException(val code: String, message: String) : Exception(message)
