package com.nije.claudesessions

import com.intellij.openapi.project.Project
import org.jetbrains.plugins.terminal.TerminalToolWindowManager

/** Opens a new terminal tab and runs the configured launch command (Settings | Tools |
 *  Claude Sessions; `claude` by default).
 *
 *  - `createLocalShellWidget` force-creates a *Classic* terminal tab regardless of the
 *    configured engine — the jump/close features depend on Classic, so this keeps every
 *    spawned session consistent with them, and returns a [ShellTerminalWidget] (the newer
 *    `createShellWidget` returns a `TerminalWidget` with no run-command method).
 *  - `executeCommand` queues the command and drains it once the shell's TerminalStarter is
 *    ready, so it's safe to call immediately after creating the widget. Running it in an
 *    interactive shell (rather than as a `-c` startup command) is what lets the command be
 *    a shell *alias* — aliases only exist inside the loaded shell. */
object ClaudeLauncher {
    @Suppress("DEPRECATION")
    fun spawn(project: Project) {
        val mgr = TerminalToolWindowManager.getInstance(project)
        val dir = project.basePath ?: System.getProperty("user.home")
        val widget = mgr.createLocalShellWidget(dir, "claude", true)
        runCatching { widget.executeCommand(ClaudeSessionsSettings.getInstance().launchCommand) }
        TerminalJump.focusTerminal(project, widget)   // raise the IDE + focus so typing lands here
    }
}
