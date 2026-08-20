package io.nomadserver.wallet.nostr

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import rust.nostr.sdk.Keys

/**
 * The wallet's Nostr identity: one long-lived keypair, generated on first
 * run, stored only in Keystore-backed EncryptedSharedPreferences
 * (THREAT_MODEL.md "Key handling"). Never logged, never leaves the device
 * except as the signing key for protocol messages.
 */
class IdentityStore(context: Context) {

    private val prefs = EncryptedSharedPreferences.create(
        context,
        "nomad_identity",
        MasterKey.Builder(context).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    /** Load the identity keypair, generating and persisting one on first run. */
    fun getOrCreate(): Keys {
        val hex = prefs.getString(KEY_SECRET, null)
        if (!hex.isNullOrEmpty()) {
            return runCatching { Keys.parse(hex) }.getOrNull() ?: regenerate()
        }
        return regenerate()
    }

    private fun regenerate(): Keys {
        val keys = Keys.generate()
        prefs.edit().putString(KEY_SECRET, keys.secretKey().toHex()).apply()
        return keys
    }

    private companion object {
        const val KEY_SECRET = "nostr_secret_hex"
    }
}
