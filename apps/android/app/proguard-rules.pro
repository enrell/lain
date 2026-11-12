# The Rust core is reached through JNA by name: its bindings, callbacks and
# structures must keep their classes, members and signatures.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class io.github.enrell.lain.core.** { *; }
-keepattributes Signature,InnerClasses,EnclosingMethod,*Annotation*
-dontwarn java.awt.**
-dontwarn com.sun.jna.platform.**
