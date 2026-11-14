# JNA and the UniFFI bindings are accessed reflectively from native code.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class uniffi.notes_ffi.** { *; }
-dontwarn java.awt.**

# ML Kit (QR scanner) creates its component registrars reflectively.
-keep class * implements com.google.firebase.components.ComponentRegistrar { <init>(); }
-keep class com.google.mlkit.**.*Registrar { <init>(); }
