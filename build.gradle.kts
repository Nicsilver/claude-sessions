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
        // Build against the installed IDE (no multi-GB SDK download).
        local("/Users/nije/Applications/IntelliJ IDEA.app")
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
