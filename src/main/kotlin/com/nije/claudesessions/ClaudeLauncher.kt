package com.nije.claudesessions

import com.intellij.openapi.project.Project
import org.jetbrains.plugins.terminal.TerminalToolWindowManager

/** Opens a new terminal tab and runs the user's `clauded` alias (their shortcut for
 *  launching a Claude Code session), optionally seeding it with an initial prompt.
 *
 *  - `createLocalShellWidget` force-creates a *Classic* terminal tab regardless of the
 *    configured engine — the jump/close features depend on Classic, so this keeps every
 *    spawned session consistent with them, and returns a [ShellTerminalWidget] (the newer
 *    `createShellWidget` returns a `TerminalWidget` with no run-command method).
 *  - `executeCommand` queues the command and drains it once the shell's TerminalStarter is
 *    ready, so it's safe to call immediately after creating the widget. Running it in an
 *    interactive shell (rather than as a `-c` startup command) is what makes the `clauded`
 *    *alias* resolve — aliases only exist inside the loaded shell.
 *  - When [prompt] is set, the session starts with that prompt already submitted
 *    (`clauded '<prompt>'`) — e.g. a `/perform-team-test AGI-xxxxx` slash command fired by
 *    `start-team-test.sh`. The tab is named `TT <issue-id>` when the prompt carries one. */
object ClaudeLauncher {
    @Suppress("DEPRECATION")
    fun spawn(project: Project, prompt: String? = null) {
        val mgr = TerminalToolWindowManager.getInstance(project)
        val dir = project.basePath ?: System.getProperty("user.home")
        val tab = prompt?.let { Regex("[A-Z]+-\\d+").find(it)?.value?.let { id -> "TT $id" } } ?: "claude"
        val widget = mgr.createLocalShellWidget(dir, tab, true)
        val command = if (prompt.isNullOrBlank()) "clauded" else "clauded ${shellQuote(prompt)}"
        runCatching { widget.executeCommand(command) }
        TerminalJump.focusTerminal(project, widget)   // raise the IDE + focus so typing lands here
    }

    /** Single-quote for the interactive shell, escaping any embedded single quote. */
    private fun shellQuote(s: String) = "'" + s.replace("'", "'\\''") + "'"
}
