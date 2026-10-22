# JNA and UniFFI's generated bindings reach classes by reflection and JNI.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class uniffi.** { *; }
-dontwarn java.awt.**
# Called from Rust through JNI.
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }
-keep class nl.merel.app.MerelNative { *; }
