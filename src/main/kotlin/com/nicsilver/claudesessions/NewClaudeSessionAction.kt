package com.nicsilver.claudesessions

import com.intellij.openapi.actionSystem.ActionUpdateThread
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent

/** "New Claude Session" — opens a terminal tab and runs the configured launch command.
 *  Ships with no default shortcut; bind one in Settings | Keymap | "New Claude Session". */
class NewClaudeSessionAction : AnAction() {
    override fun actionPerformed(e: AnActionEvent) {
        e.project?.let { ClaudeLauncher.spawn(it) }
    }

    override fun update(e: AnActionEvent) {
        e.presentation.isEnabledAndVisible = e.project != null
    }

    override fun getActionUpdateThread() = ActionUpdateThread.BGT
}
