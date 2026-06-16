package com.nije.claudesessions

import com.google.gson.JsonParser
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.ProjectActivity
import kotlinx.coroutines.delay
import java.io.File

/** Watches ~/.claude/session-status/focus-request.json (written by the external Swift
 *  panels on click) and jumps to the matching terminal tab, bringing the IDE forward.
 *  Each open project runs this; only the one actually holding the matching tty acts. */
class FocusWatcher : ProjectActivity {
    override suspend fun execute(project: Project) {
        val file = File(System.getProperty("user.home"), ".claude/session-status/focus-request.json")
        var lastTs = 0.0
        while (true) {
            delay(400)
            val req = read(file) ?: continue
            if (req.second <= lastTs) continue
            lastTs = req.second
            val tty = req.first
            ApplicationManager.getApplication().invokeLater {
                TerminalJump.jumpToTty(project, tty, bringToFront = true)
            }
        }
    }

    private fun read(f: File): Pair<String, Double>? = try {
        if (!f.exists()) null else {
            val o = JsonParser.parseString(f.readText()).asJsonObject
            val tty = if (o.has("tty") && !o.get("tty").isJsonNull) o.get("tty").asString else ""
            val ts = if (o.has("ts") && !o.get("ts").isJsonNull) o.get("ts").asDouble else 0.0
            if (tty.isEmpty()) null else tty to ts
        }
    } catch (e: Throwable) {
        null
    }
}
