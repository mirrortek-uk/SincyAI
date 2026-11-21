# JNI: AdapterRuntime native methods + the classes JNI looks up by name must survive
-keep class com.sincy.agent.** { *; }
-keepclasseswithmembernames class * { native <methods>; }
# OkHttp / coroutines / serialization keep their own consumer rules
