package nl.merel.app

import android.content.Context

/** The one call into Merel's core that is not part of its UniFFI interface. */
object MerelNative {
    init {
        System.loadLibrary("merel_mobile")
    }

    /** Lets the core check server certificates with Android's own trust store; call before connecting. */
    @JvmStatic
    external fun init(context: Context)
}
