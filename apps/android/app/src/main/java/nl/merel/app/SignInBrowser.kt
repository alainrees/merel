package nl.merel.app

import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import java.lang.ref.WeakReference
import uniffi.merel_mobile.SignInBrowser

/**
 * Opens a sign-in link in a Custom Tab over Merel. Microsoft or Google send the person back to
 * `http://127.0.0.1`, where Merel's engine listens for this one sign-in, so the app never sees
 * the password; the store closes the tab when it is done.
 */
object CustomTabSignIn : SignInBrowser {
    /** The activity the tab opens over, set when it starts. */
    @Volatile
    var activity: WeakReference<Activity> = WeakReference(null)

    override fun show(url: String): Boolean {
        val uri = Uri.parse(url)
        val shown = activity.get() ?: return false
        if (uri.scheme != "https") return false
        shown.runOnUiThread {
            runCatching { CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(shown, uri) }
                .onFailure { runCatching { shown.startActivity(Intent(Intent.ACTION_VIEW, uri)) } }
        }
        return true
    }

    /** Brings Merel back over the tab, which closes it. */
    fun close() {
        val shown = activity.get() ?: return
        shown.runOnUiThread {
            shown.startActivity(
                Intent(shown, shown.javaClass).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            )
        }
    }
}
