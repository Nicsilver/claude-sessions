plugins {
    id("java")
    id("org.jetbrains.kotlin.jvm") version "2.3.0"
    id("org.jetbrains.intellij.platform") version "2.16.0"
}

group = "com.nije"
version = "0.1.0"

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
            // Bare "2026.1" has a metadata entry but no downloadable artifact; the GA download is
            // the patch release. 2026.1.4 is build 261.x, within the sinceBuild/untilBuild range.
            intellijIdeaCommunity("2026.1.4")
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
