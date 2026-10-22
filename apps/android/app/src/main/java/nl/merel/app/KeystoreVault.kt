package nl.merel.app

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.merel_mobile.SecretVault
import uniffi.merel_mobile.VaultException

/**
 * Merel's secrets (passwords, tokens, the offline cache's key), sealed with AES-GCM under a key
 * that lives in the Android Keystore and never leaves this phone. Only the sealed bytes are in
 * the app's private preferences, which are left out of every backup.
 */
class KeystoreVault(context: Context) : SecretVault {
    private val prefs = context.getSharedPreferences("merel-vault", Context.MODE_PRIVATE)

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    override fun get(key: String): String? = guard {
        val sealed = prefs.getString(key, null) ?: return@guard null
        val bytes = Base64.decode(sealed, Base64.NO_WRAP)
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, IV))
        cipher.updateAAD(key.encodeToByteArray())
        String(cipher.doFinal(bytes, IV, bytes.size - IV), Charsets.UTF_8)
    }

    override fun set(key: String, secret: String) = guard {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        cipher.updateAAD(key.encodeToByteArray())
        val sealed = cipher.iv + cipher.doFinal(secret.encodeToByteArray())
        check(cipher.iv.size == IV)
        if (!prefs.edit().putString(key, Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()) error("not saved")
    }

    override fun delete(key: String) = guard {
        if (!prefs.edit().remove(key).commit()) error("not removed")
    }

    private fun <T> guard(block: () -> T): T =
        try {
            block()
        } catch (e: Exception) {
            throw VaultException.Refused(e.javaClass.simpleName)
        }

    private companion object {
        const val ALIAS = "merel-vault"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV = 12
    }
}
