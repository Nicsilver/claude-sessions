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

    /** Close the terminal tab whose shell runs on a given tty — terminates the shell
     *  (and the Claude process inside it) and removes the tab. Only the project that
     *  actually holds the matching tty acts; others no-op. */
    fun closeTty(project: Project, tty: String) {
        if (tty.isEmpty()) return
        val mgr = TerminalToolWindowManager.getInstance(project)
        val widgets = runCatching { mgr.terminalWidgets }.getOrNull() ?: emptySet()
        val w = widgets.firstOrNull { val t = widgetTty(it); t.isNotEmpty() && norm(t) == norm(tty) }
        if (w == null) { dbg("closeTty tty='$tty' widgets=${widgets.size} -> <none>"); return }
        val content = runCatching { mgr.getContainer(w)?.content }.getOrNull()
        if (content == null) { dbg("closeTty tty='$tty' -> no content"); return }
        dbg("closeTty tty='$tty' content='${content.displayName}' -> closeTab")
        // closeTab() is the terminal's own tab-close (same as the × button): terminates
        // the session and removes the tab. removeContent() on the tool window's content
        // manager silently no-ops here.
        runCatching { mgr.closeTab(content) }.onFailure { dbg("  closeTab failed: ${it.message}") }
    }

    /** Bring the IDE forward and focus a freshly-spawned terminal widget so the user can
     *  type immediately. Needed because the spawn is triggered from an external panel, so
     *  the IDE isn't frontmost and the widget's in-IDE focus request alone isn't enough. */
    fun focusTerminal(project: Project, widget: Any?) {
        if (widget == null) return
        val tw = ToolWindowManager.getInstance(project).getToolWindow("Terminal") ?: return
        tw.activate({
            activateThisApp()
            val comp = focusableOf(widget) ?: componentOf(widget)
            val win = comp?.let { javax.swing.SwingUtilities.getWindowAncestor(it) }
            win?.toFront()
            win?.requestFocus()
            com.intellij.openapi.application.ApplicationManager.getApplication().invokeLater {
                if (comp != null) {
                    com.intellij.openapi.wm.IdeFocusManager.getInstance(project).requestFocus(comp, true)
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
     *  IntelliJ instance), letting its window become key for immediate typing.
     *
     *  Prefers the native `activate-pid` helper (NSRunningApplication.activate ~10ms,
     *  consistent); falls back to osascript→System Events (~100ms, and spikes to 1–2s
     *  when System Events is cold/busy) only if the helper isn't built. */
    private fun activateThisApp() {
        runCatching {
            val pid = ProcessHandle.current().pid()
            val home = System.getProperty("user.home")
            val helper = sequenceOf(
                "$home/IdeaProjects/claude-sessions/session-status/bin/activate-pid",
                "$home/.claude/session-status/bin/activate-pid",
            ).map(::File).firstOrNull { it.canExecute() }
            if (helper != null) {
                ProcessBuilder(helper.path, "$pid").start()
            } else {
                ProcessBuilder(
                    "osascript", "-e",
                    "tell application \"System Events\" to set frontmost of (first process whose unix id is $pid) to true"
                ).start()
            }
        }
    }

    /** Public accessor for the controlling tty of a terminal widget (e.g. "ttys004"),
     *  "" if none — used by [TabNamePublisher] to join tabs to recorder sessions. */
    fun ttyOf(widget: Any): String = widgetTty(widget)

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

    /** pid → controlling tty. A shell's tty never changes, so resolve once and cache.
     *  Resolving forks `ps`, which can stall 100ms+ under memory pressure — forking on the
     *  EDT froze the whole IDE (typing stutter every publish tick), so a cache miss on the
     *  EDT resolves on a pooled thread instead and callers pick the value up next tick. */
    private val ttyCache = java.util.concurrent.ConcurrentHashMap<Long, String>()
    private val ttyPending = java.util.concurrent.ConcurrentHashMap.newKeySet<Long>()

    private fun ttyOfPid(pid: Long): String {
        ttyCache[pid]?.let { return it }
        val app = com.intellij.openapi.application.ApplicationManager.getApplication()
        if (app.isDispatchThread) {
            if (ttyPending.add(pid)) {
                app.executeOnPooledThread {
                    try { lookupTty(pid)?.let { ttyCache[pid] = it } } finally { ttyPending.remove(pid) }
                }
            }
            return ""
        }
        return lookupTty(pid)?.also { ttyCache[pid] = it } ?: ""
    }

    private fun lookupTty(pid: Long): String? = try {
        val p = ProcessBuilder("ps", "-o", "tty=", "-p", "$pid").redirectErrorStream(true).start()
        val out = p.inputStream.bufferedReader().readText().trim()
        if (out.isEmpty() || out == "??" || out == "?") null else out
    } catch (e: Exception) {
        null
    }

    private fun norm(tty: String) = tty.removePrefix("tty")
}
