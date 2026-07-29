package com.nicsilver.claudesessions

import com.intellij.openapi.options.BoundConfigurable
import com.intellij.openapi.ui.DialogPanel
import com.intellij.ui.dsl.builder.COLUMNS_MEDIUM
import com.intellij.ui.dsl.builder.bindText
import com.intellij.ui.dsl.builder.columns
import com.intellij.ui.dsl.builder.panel

class ClaudeSessionsConfigurable : BoundConfigurable("Claude Sessions") {

    override fun createPanel(): DialogPanel = panel {
        row("Launch command:") {
            textField()
                .columns(COLUMNS_MEDIUM)
                .bindText(
                    { ClaudeSessionsSettings.getInstance().launchCommand },
                    { ClaudeSessionsSettings.getInstance().launchCommand = it },
                )
                .comment(
                    "Run by <b>Tools | New Claude Session</b> in a fresh terminal tab. " +
                        "It runs inside an interactive shell, so a shell alias works here too.",
                )
        }
    }
}
