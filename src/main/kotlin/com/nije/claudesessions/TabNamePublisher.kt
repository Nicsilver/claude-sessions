package com.nije.claudesessions

import com.google.gson.Gson
import com.intellij.openapi.project.Project
import org.jetbrains.plugins.terminal.TerminalToolWindowManager
import java.io.File

/** Publishes a `tty → terminal-tab-name` map so the external session-status recorder can
 *  label a session by its IDE tab (e.g. "TT AGI-18033") instead of a generic AI title.
 *
 *  One file per project (keyed by [Project.getLocationHash]) under
 *  `~/.claude/session-status/tab-names/`, rewritten every couple of seconds by the
 *  [FocusWatcher] loop. Each entry carries a `ts` so the recorder can ignore stale entries
 *  from a project that has since closed. Default `Local` / `Local (N)` tab titles are
 *  published too — the recorder owns the rule that those don't count as a real label. */
object TabNamePublisher {
    private val gson = Gson()
    private val dir = File(System.getProperty("user.home"), ".claude/session-status/tab-names")

    fun publish(project: Project) {
        val mgr = TerminalToolWindowManager.getInstance(project)
        val widgets = runCatching { mgr.terminalWidgets }.getOrNull() ?: return
        val now = System.currentTimeMillis() / 1000.0
        val map = HashMap<String, Map<String, Any>>()
        for (w in widgets) {
            val tty = TerminalJump.ttyOf(w)
            if (tty.isEmpty()) continue
            val name = runCatching { mgr.getContainer(w)?.content?.displayName }.getOrNull()?.trim().orEmpty()
            // auto = the TabNamer set this name (an echo of the session topic, not a label
            // the user chose) — the recorder skips auto names when deriving tab_title.
            if (name.isNotEmpty()) {
                map[tty] = mapOf("name" to name, "ts" to now, "auto" to (name == TabNamer.appliedName(tty)))
            }
        }
        runCatching {
            dir.mkdirs()
            val out = File(dir, project.locationHash + ".json")
            val tmp = File(dir, project.locationHash + ".json.tmp")
            tmp.writeText(gson.toJson(map))
            tmp.renameTo(out)
        }
    }
}
