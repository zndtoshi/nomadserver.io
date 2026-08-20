package io.nomadserver.wallet.pairing

import io.nomadserver.wallet.nostr.PairingPayload
import io.nomadserver.wallet.nostr.protocolJson
import java.util.Base64
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * Pairing payload validation + proof (PROTOCOL.md §4). The QR payload
 * authorizes one pairing; the wallet proves possession of the secret with
 * HMAC-SHA256(key = pairSecret, msg = wallet pubkey hex).
 */
object Pairing {

    class Invalid(message: String) : Exception(message)

    fun parsePayload(raw: String, nowSecs: Long): PairingPayload {
        val p = runCatching {
            protocolJson.decodeFromString(PairingPayload.serializer(), raw)
        }.getOrElse { throw Invalid("not a Nomad pairing payload") }

        if (p.app != "nomad-server") throw Invalid("wrong app: ${p.app}")
        if (p.v != 1) throw Invalid("unsupported version ${p.v}")
        if (!p.nodePubkey.matches(Regex("[0-9a-f]{64}"))) throw Invalid("bad server pubkey")
        if (p.relays.isEmpty()) throw Invalid("no relays")
        if (p.relays.any { !it.startsWith("wss://") }) throw Invalid("bad relay url")
        if (p.exp <= nowSecs) throw Invalid("pairing code expired — reload the QR on the server")
        return p
    }

    fun proof(pairSecretB64Url: String, walletPubkeyHex: String): String {
        val secret = Base64.getUrlDecoder().decode(pairSecretB64Url)
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(secret, "HmacSHA256"))
        return mac.doFinal(walletPubkeyHex.toByteArray(Charsets.UTF_8)).toHex()
    }

    private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
}
