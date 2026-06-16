package com.nije.claudesessions

import com.google.gson.JsonParser
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.ProjectActivity
import kotlinx.coroutines.delay
import java.io.File

/** Watches ~/.claude/session-status/focus-request.json (written by the external Swift
 *  panels) and acts on the matching terminal tab: a left-click "focus" request jumps to
 *  the tab (raising the IDE); a right-click "close" request closes the tab (terminating
 *  the session). Each open project runs this; only the one holding the matching tty acts. */
class FocusWatcher : ProjectActivity {
    private data class Req(val tty: String, val ts: Double, val action: String)

    override suspend fun execute(project: Project) {
        val file = File(System.getProperty("user.home"), ".claude/session-status/focus-request.json")
        var lastTs = 0.0
        while (true) {
            delay(400)
            val req = read(file) ?: continue
            if (req.ts <= lastTs) continue
            lastTs = req.ts
            ApplicationManager.getApplication().invokeLater {
                if (req.action == "close") TerminalJump.closeTty(project, req.tty)
                else TerminalJump.jumpToTty(project, req.tty, bringToFront = true)
            }
        }
    }

    private fun read(f: File): Req? = try {
        if (!f.exists()) null else {
            val o = JsonParser.parseString(f.readText()).asJsonObject
            val tty = if (o.has("tty") && !o.get("tty").isJsonNull) o.get("tty").asString else ""
            val ts = if (o.has("ts") && !o.get("ts").isJsonNull) o.get("ts").asDouble else 0.0
            val action = if (o.has("action") && !o.get("action").isJsonNull) o.get("action").asString else "focus"
            if (tty.isEmpty()) null else Req(tty, ts, action)
        }
    } catch (e: Throwable) {
        null
    }
}
