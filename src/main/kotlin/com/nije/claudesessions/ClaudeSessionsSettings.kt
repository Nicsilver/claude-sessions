package com.nije.claudesessions

import com.intellij.openapi.components.PersistentStateComponent
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.State
import com.intellij.openapi.components.Storage
import com.intellij.openapi.components.service

/** Application-level settings, shown under Settings | Tools | Claude Sessions. */
@Service(Service.Level.APP)
@State(name = "ClaudeSessionsSettings", storages = [Storage("claude-sessions.xml")])
class ClaudeSessionsSettings : PersistentStateComponent<ClaudeSessionsSettings.State> {

    class State {
        @JvmField
        var launchCommand: String = DEFAULT_LAUNCH_COMMAND
    }

    private var state = State()

    override fun getState(): State = state

    override fun loadState(state: State) {
        this.state = state
    }

    /** A blank field means "I never set this", not "run nothing" — hand back the default so
     *  clearing the setting can't leave the action silently doing nothing. */
    var launchCommand: String
        get() = state.launchCommand.ifBlank { DEFAULT_LAUNCH_COMMAND }
        set(value) {
            state.launchCommand = value.trim()
        }

    companion object {
        const val DEFAULT_LAUNCH_COMMAND = "claude"

        fun getInstance(): ClaudeSessionsSettings = service()
    }
}
