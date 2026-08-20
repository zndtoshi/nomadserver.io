package io.nomadserver.wallet.watch

import org.bitcoindevkit.Descriptor
import org.bitcoindevkit.Network
import org.bitcoindevkit.NetworkKind

/**
 * Extended-pubkey handling for watch-only targets.
 *
 * The wallet derives addresses from xpub/ypub/zpub (and testnet variants)
 * itself — the server only ever sees plain address lists (ARCHITECTURE.md:
 * "the server is a dumb chain proxy"). Derivation uses BDK descriptors;
 * no private material exists anywhere in this app.
 */
object Deriver {

    class Invalid(message: String) : Exception(message)

    // base58check version bytes for extended pubkeys
    private val VERSIONS = mapOf(
        "xpub" to "0488b21e", "ypub" to "0295b43f", "zpub" to "02aa7ed3",
        "tpub" to "043587cf", "upub" to "044a5262", "vpub" to "045f1cf6",
    )

    /** Receive-chain addresses to derive for a fresh target. */
    private const val RECEIVE_COUNT = 20
    private const val CHANGE_COUNT = 10

    fun isExtendedPubkey(input: String): Boolean =
        VERSIONS.keys.any { input.startsWith(it) } && input.length in 100..120

    /**
     * Derive watch addresses for an extended pubkey. ypub/zpub-style
     * prefixes are converted to the xpub form with the right descriptor
     * wrapper (ypub → sh(wpkh()), zpub → wpkh()).
     */
    fun deriveAddresses(xpubInput: String, mainnet: Boolean): List<String> {
        val prefix = VERSIONS.keys.firstOrNull { xpubInput.startsWith(it) }
            ?: throw Invalid("not an extended pubkey")
        val (xpub, wrapped) = when (prefix) {
            "ypub" -> convertVersion(xpubInput, "0488b21e") to true
            "zpub" -> convertVersion(xpubInput, "0488b21e") to false
            "upub" -> convertVersion(xpubInput, "043587cf") to true
            "vpub" -> convertVersion(xpubInput, "043587cf") to false
            else -> xpubInput to false // xpub/tpub
        }
        val keyOriginFree = xpub.trim()
        val kind = if (mainnet) NetworkKind.MAIN else NetworkKind.TEST
        val network = if (mainnet) Network.BITCOIN else Network.TESTNET

        val out = ArrayList<String>(RECEIVE_COUNT + CHANGE_COUNT)
        for ((chain, count) in listOf(0 to RECEIVE_COUNT, 1 to CHANGE_COUNT)) {
            val inner = "wpkh($keyOriginFree/$chain/*)"
            val descStr = if (wrapped) "sh($inner)" else inner
            val desc = try {
                Descriptor(descStr, kind)
            } catch (e: Exception) {
                throw Invalid("descriptor rejected: ${e.message}")
            }
            for (i in 0 until count) {
                out.add(desc.deriveAddress(i.toUInt(), network).toString())
            }
        }
        return out
    }

    /** Swap base58check version bytes (payload unchanged). */
    private fun convertVersion(xpub: String, newVersionHex: String): String {
        val raw = base58CheckDecode(xpub)
        val payload = raw.copyOfRange(4, raw.size)
        val version = newVersionHex.hexToBytes()
        return base58CheckEncode(version + payload)
    }

    private fun base58CheckDecode(s: String): ByteArray {
        val alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
        var num = java.math.BigInteger.ZERO
        for (c in s) {
            val digit = alphabet.indexOf(c)
            if (digit < 0) throw Invalid("invalid base58")
            num = num.multiply(java.math.BigInteger.valueOf(58)).add(java.math.BigInteger.valueOf(digit.toLong()))
        }
        var bytes = num.toByteArray()
        if (bytes.isNotEmpty() && bytes[0] == 0.toByte()) bytes = bytes.copyOfRange(1, bytes.size)
        val leadingZeros = s.takeWhile { it == '1' }.length
        val full = ByteArray(leadingZeros) + bytes
        if (full.size < 4) throw Invalid("too short")
        val (payload, checksum) = full.copyOfRange(0, full.size - 4) to full.copyOfRange(full.size - 4, full.size)
        val digest = sha256(sha256(payload))
        if (!digest.copyOfRange(0, 4).contentEquals(checksum)) throw Invalid("bad base58check checksum")
        return payload
    }

    private fun base58CheckEncode(payload: ByteArray): String {
        val alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
        val checksum = sha256(sha256(payload)).copyOfRange(0, 4)
        val full = payload + checksum
        var num = java.math.BigInteger(1, full)
        val sb = StringBuilder()
        while (num > java.math.BigInteger.ZERO) {
            val (q, r) = num.divideAndRemainder(java.math.BigInteger.valueOf(58))
            sb.append(alphabet[r.toInt()])
            num = q
        }
        for (b in full) {
            if (b == 0.toByte()) sb.append('1') else break
        }
        return sb.reverse().toString()
    }

    private fun sha256(b: ByteArray): ByteArray =
        java.security.MessageDigest.getInstance("SHA-256").digest(b)

    private fun String.hexToBytes(): ByteArray =
        chunked(2).map { it.toInt(16).toByte() }.toByteArray()
}
