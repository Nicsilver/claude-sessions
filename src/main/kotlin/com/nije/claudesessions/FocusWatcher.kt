package com.nije.claudesessions

import com.google.gson.JsonParser
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.project.ProjectManager
import com.intellij.openapi.startup.ProjectActivity
import com.intellij.openapi.wm.WindowManager
import kotlinx.coroutines.delay
import java.io.File

/** Watches ~/.claude/session-status/focus-request.json (written by the external Swift
 *  panels) and acts on the matching terminal tab: a left-click "focus" request jumps to
 *  the tab (raising the IDE); a right-click "close" request closes the tab (terminating
 *  the session). Each open project runs this; only the one holding the matching tty acts. */
class FocusWatcher : ProjectActivity {
    private data class Req(val tty: String, val ts: Double, val action: String, val cmds: List<String>)

    override suspend fun execute(project: Project) {
        val file = File(System.getProperty("user.home"), ".claude/session-status/focus-request.json")
        var lastTs = 0.0
        var tick = 0
        while (true) {
            delay(100)   // was 400ms; tighter poll so a click registers near-instantly
            // Every ~2s: name tabs after their sessions, then republish the tty→tab-name map
            // for the recorder.
            if (tick++ % 20 == 0) {
                ApplicationManager.getApplication().invokeLater {
                    TabNamer.apply(project)
                    TabNamePublisher.publish(project)
                }
            }
            val req = read(file) ?: continue
            if (req.ts <= lastTs) continue
            lastTs = req.ts
            if (req.action == "team-test") {
                // `start-team-test.sh` drops one request carrying N `/perform-team-test`
                // commands. Spawn one Claude session per command, staggered so five Chrome
                // lanes don't all launch in the same instant. Staleness guard: ignore a
                // request left over from a past run, so a whole batch can't replay when the
                // IDE next starts (lastTs resets to 0 and would re-fire the stored request).
                val ageSec = (System.currentTimeMillis() / 1000.0) - req.ts
                if (ageSec !in 0.0..60.0) continue
                for ((i, cmd) in req.cmds.withIndex()) {
                    if (i > 0) delay(300)
                    ApplicationManager.getApplication().invokeLater {
                        if (isMostRecentProject(project)) ClaudeLauncher.spawn(project, cmd)
                    }
                }
                continue
            }
            ApplicationManager.getApplication().invokeLater {
                when (req.action) {
                    "close" -> TerminalJump.closeTty(project, req.tty)
                    "new"   -> if (isMostRecentProject(project)) ClaudeLauncher.spawn(project)
                    else    -> TerminalJump.jumpToTty(project, req.tty, bringToFront = true)
                }
            }
        }
    }

    /** A "new session" / "team-test" request carries no tty, so every open project sees
     *  it. Elect a single one — the most-recently-focused project's frame (or, failing
     *  that, the first open project) — so exactly one spawns. */
    private fun isMostRecentProject(project: Project): Boolean {
        val wm = WindowManager.getInstance()
        val recent = wm.mostRecentFocusedWindow
        val open = ProjectManager.getInstance().openProjects
        val target = open.firstOrNull { wm.getFrame(it) === recent } ?: open.firstOrNull()
        return project === target
    }

    private fun read(f: File): Req? = try {
        if (!f.exists()) null else {
            val o = JsonParser.parseString(f.readText()).asJsonObject
            val tty = if (o.has("tty") && !o.get("tty").isJsonNull) o.get("tty").asString else ""
            val ts = if (o.has("ts") && !o.get("ts").isJsonNull) o.get("ts").asDouble else 0.0
            val action = if (o.has("action") && !o.get("action").isJsonNull) o.get("action").asString else "focus"
            val cmds = if (o.has("cmds") && o.get("cmds").isJsonArray)
                o.getAsJsonArray("cmds").mapNotNull { if (it.isJsonNull) null else it.asString } else emptyList()
            // "new"/"team-test" carry no tty; every other action needs one.
            if (tty.isEmpty() && action != "new" && action != "team-test") null else Req(tty, ts, action, cmds)
        }
    } catch (e: Throwable) {
        null
    }
}
