plugins {
    id("java")
    id("org.jetbrains.kotlin.jvm") version "2.3.0"
    id("org.jetbrains.intellij.platform") version "2.18.1"
}

group = "com.nije"
version = "0.3.0"

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    // Platform provides gson at runtime; compileOnly just satisfies the compiler.
    compileOnly("com.google.code.gson:gson:2.11.0")
    intellijPlatform {
        // Locally, build against the installed IDE (no multi-GB SDK download). On CI (or any
        // machine without that install) fall back to downloading the Community SDK, which Gradle
        // caches. Override the local path with -PlocalIde=/path/to/IDE if yours differs.
        val localIde = file(providers.gradleProperty("localIde")
            .getOrElse("/Users/nije/Applications/IntelliJ IDEA.app"))
        if (localIde.exists()) {
            local(localIde)
        } else {
            // useInstaller = false fetches the ideaIC artifact from the intellij-repository Maven
            // repo instead of the data.services installer feed — the feed lags and doesn't list
            // 2026.1.4 yet, but the Maven repo has it. 2026.1.4 is build 261.x (matches sinceBuild).
            intellijIdeaCommunity("2026.1.4") { useInstaller = false }
            // The multi-OS archive (useInstaller=false) doesn't bundle the JetBrains Runtime, so
            // add it explicitly — otherwise runtimeDirectory resolution fails.
            jetbrainsRuntime()
        }
        bundledPlugin("org.jetbrains.plugins.terminal")
    }
}

intellijPlatform {
    pluginConfiguration {
        ideaVersion {
            sinceBuild = "261"
            untilBuild = "261.*"
        }
    }
}

kotlin {
    jvmToolchain(21)
}

tasks {
    // Optional task that boots a headless IDE to pre-index Settings search; we
    // contribute no settings, and it trips on a JBR bootstrap quirk — skip it.
    buildSearchableOptions {
        enabled = false
    }
}
