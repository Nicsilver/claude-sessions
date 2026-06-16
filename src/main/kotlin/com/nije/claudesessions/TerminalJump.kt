package com.nije.claudesessions

import com.intellij.ide.impl.ProjectUtil
import com.intellij.openapi.project.Project
import com.intellij.openapi.wm.ToolWindowManager
import org.jetbrains.plugins.terminal.TerminalToolWindowManager
import java.io.File

/** Selects the (classic) terminal tab whose shell runs on a given tty, optionally
 *  pulling the IDE window to the front. Shared by the tool window (double-click) and
 *  the file watcher (clicks from the external Swift panels). */
object TerminalJump {
    private val debugLog = File(System.getProperty("user.home"), ".claude/session-status/plugin-debug.log")
    private fun dbg(m: String) { try { debugLog.appendText("$m\n") } catch (e: Throwable) { /* ignore */ } }

    fun jumpToTty(project: Project, tty: String, bringToFront: Boolean) {
        if (tty.isEmpty()) return
        val tw = ToolWindowManager.getInstance(project).getToolWindow("Terminal") ?: return
        val cm = tw.contentManager
        // Enumerate ALL widgets (a split tab holds several panes in one Content), match
        // the exact pane by tty, map it to its Content via getContainer(), select that
        // tab, then focus that specific pane.
        val mgr = TerminalToolWindowManager.getInstance(project)
        val widgets = runCatching { mgr.terminalWidgets }.getOrNull() ?: emptySet()
        val targetWidget = widgets.firstOrNull { val t = widgetTty(it); t.isNotEmpty() && norm(t) == norm(tty) }
        if (targetWidget == null) { dbg("jumpToTty tty='$tty' widgets=${widgets.size} -> <none>"); return }
        val comp = componentOf(targetWidget)
        val targetContent = runCatching { mgr.getContainer(targetWidget)?.content }.getOrNull()
        dbg("jumpToTty tty='$tty' widgets=${widgets.size} content='${targetContent?.displayName}' comp=${comp?.javaClass?.simpleName} (front=$bringToFront)")
        tw.activate({
            // Activate THIS app (by pid, never the wrong IntelliJ), select the tab, raise
            // its window (its own floating window in window mode), then focus the specific
            // pane via the IDE focus manager so you can type immediately.
            targetContent?.let { cm.setSelectedContent(it, true) }
            if (bringToFront) {
                activateThisApp()
                val win = javax.swing.SwingUtilities.getWindowAncestor(comp ?: targetContent?.component)
                win?.toFront()
                win?.requestFocus()
                com.intellij.openapi.application.ApplicationManager.getApplication().invokeLater {
                    val f = focusableOf(targetWidget) ?: comp
                    if (f != null) {
                        com.intellij.openapi.wm.IdeFocusManager.getInstance(project).requestFocus(f, true)
                        dbg("  focused ${f.javaClass.simpleName}")
                    }
                }
            }
        }, true, true)
    }

    // TerminalWidget extends ComponentContainer, so get its component/focusable directly.
    private fun componentOf(w: Any): java.awt.Component? =
        (w as? com.intellij.openapi.ui.ComponentContainer)?.component ?: (w as? java.awt.Component)

    private fun focusableOf(w: Any): java.awt.Component? =
        (w as? com.intellij.openapi.ui.ComponentContainer)?.preferredFocusableComponent ?: componentOf(w)

    /** Bring THIS IDE app to the front by its own pid (so we never raise the wrong
     *  IntelliJ instance), letting its window become key for immediate typing. */
    private fun activateThisApp() {
        runCatching {
            val pid = ProcessHandle.current().pid()
            ProcessBuilder(
                "osascript", "-e",
                "tell application \"System Events\" to set frontmost of (first process whose unix id is $pid) to true"
            ).start()
        }
    }

    private fun widgetTty(w: Any): String {
        val tc = runCatching { w.javaClass.getMethod("getTtyConnector").invoke(w) }.getOrNull() ?: return ""
        val proc = processOfConnector(tc) ?: return ""
        return ttyOfPid(proc.pid())
    }

    private fun processOfConnector(connector: Any): Process? {
        runCatching {
            val tcClass = Class.forName("com.jediterm.terminal.TtyConnector")
            val ptc = Class.forName("org.jetbrains.plugins.terminal.ShellTerminalWidget")
                .getMethod("getProcessTtyConnector", tcClass).invoke(null, connector)
            val p = ptc?.let { it.javaClass.getMethod("getProcess").invoke(it) as? Process }
            if (p != null) return p
        }
        var conn: Any? = connector
        var depth = 0
        while (conn != null && depth < 8) {
            val getProcess = conn.javaClass.methods.firstOrNull {
                it.name == "getProcess" && it.parameterCount == 0 && Process::class.java.isAssignableFrom(it.returnType)
            }
            if (getProcess != null) {
                getProcess.isAccessible = true
                (getProcess.invoke(conn) as? Process)?.let { return it }
            }
            val next = conn.javaClass.methods.firstOrNull { it.name == "getConnector" && it.parameterCount == 0 } ?: break
            conn = next.invoke(conn)
            depth++
        }
        return null
    }

    private fun ttyOfPid(pid: Long): String = try {
        val p = ProcessBuilder("ps", "-o", "tty=", "-p", "$pid").redirectErrorStream(true).start()
        val out = p.inputStream.bufferedReader().readText().trim()
        if (out == "??" || out == "?") "" else out
    } catch (e: Exception) {
        ""
    }

    private fun norm(tty: String) = tty.removePrefix("tty")
}
