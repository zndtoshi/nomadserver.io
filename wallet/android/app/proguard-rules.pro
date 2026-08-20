# nostr-sdk / bdk ship native .so libraries; keep their symbols
-keep class org.rust-nostr.** { *; }
-keep class uniffi.** { *; }
-keep class org.bitcoindevkit.** { *; }
-keepclasseswithmembernames class * { native <methods>; }
