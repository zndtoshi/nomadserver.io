package io.nomadserver.wallet.nostr

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject

/**
 * Protocol envelope types — mirror shared/schemas/v1/ (canonical) and
 * docs/PROTOCOL.md. Any change here is a protocol change: keep schemas
 * and PROTOCOL.md in sync.
 */

const val PROTOCOL_VERSION = 1
const val KIND_REQUEST: UShort = 25078u
const val KIND_RESPONSE: UShort = 25079u
const val KIND_NOTIFY: UShort = 25080u

/** Gift-wrap expiration: 9 days from randomized created_at (PROTOCOL §2.2). */
const val WRAP_EXPIRATION_SECS: Long = 9 * 24 * 3600

/** Backfill window for subscriptions (created_at randomization + margin). */
const val BACKFILL_SECS: Long = 3 * 24 * 3600

@Serializable
data class RequestEnvelope(
    val v: Int = PROTOCOL_VERSION,
    val id: String,
    val ts: Long,
    val type: String,
    val payload: JsonObject,
)

@Serializable
data class ErrorBody(
    val code: String,
    val message: String,
)

@Serializable
data class ResponseEnvelope(
    val v: Int,
    val id: String,
    val ok: Boolean,
    val result: JsonObject? = null,
    val error: ErrorBody? = null,
)

@Serializable
data class NotifyEnvelope(
    val v: Int,
    val id: String,
    val ts: Long,
    val type: String,
    val payload: JsonObject,
)

@Serializable
data class PairingPayload(
    val v: Int,
    val app: String,
    val nodePubkey: String,
    val relays: List<String>,
    val pairSecret: String,
    val exp: Long,
)

val protocolJson = Json {
    ignoreUnknownKeys = true // forward compatibility (PROTOCOL.md §3)
    encodeDefaults = true
}
