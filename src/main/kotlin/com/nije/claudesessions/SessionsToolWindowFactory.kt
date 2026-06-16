package com.nije.claudesessions

import com.intellij.ide.DataManager
import com.intellij.openapi.actionSystem.DataContext
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
import com.intellij.openapi.wm.ToolWindowManager
import com.intellij.terminal.ui.TerminalWidget
import com.intellij.ui.content.Content
import com.intellij.ui.content.ContentFactory
import com.intellij.ui.components.JBList
import com.intellij.ui.components.JBScrollPane
import java.awt.BorderLayout
import java.awt.Color
import java.awt.Component
import java.awt.FlowLayout
import java.awt.event.MouseAdapter
import java.awt.event.MouseEvent
import java.io.File
import javax.swing.DefaultListCellRenderer
import javax.swing.DefaultListModel
import javax.swing.JButton
import javax.swing.JLabel
import javax.swing.JList
import javax.swing.JPanel
import javax.swing.Timer

class SessionsToolWindowFactory : ToolWindowFactory {
    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val panel = SessionsPanel(project)
        val content = ContentFactory.getInstance().createContent(panel, "", false)
        toolWindow.contentManager.addContent(content)
    }
}

class SessionsPanel(private val project: Project) : JPanel(BorderLayout()) {
    private val log = Logger.getInstance(SessionsPanel::class.java)
    private val model = DefaultListModel<Session>()
    private val list = JBList(model)

    init {
        list.cellRenderer = SessionRenderer()
        list.addMouseListener(object : MouseAdapter() {
            override fun mouseClicked(e: MouseEvent) {
                if (e.clickCount == 2) list.selectedValue?.let { focus(it) }
            }
        })

        val toolbar = JPanel(FlowLayout(FlowLayout.LEFT))
        toolbar.add(JButton("New Claude session").apply { addActionListener { newTerminal() } })
        toolbar.add(JButton("Refresh").apply { addActionListener { refresh() } })
        add(toolbar, BorderLayout.NORTH)
        add(JBScrollPane(list), BorderLayout.CENTER)

        refresh()
        Timer(1500) { refresh() }.apply { isRepeats = true; start() }
    }

    private fun refresh() {
        val keep = list.selectedValue?.sessionId
        model.clear()
        StateReader.load().forEach { model.addElement(it) }
        if (keep != null) {
            for (i in 0 until model.size()) {
                if (model.get(i).sessionId == keep) { list.selectedIndex = i; break }
            }
        }
    }

    private fun newTerminal() = ClaudeLauncher.spawn(project)

    private val debugLog = java.io.File(System.getProperty("user.home"), ".claude/session-status/plugin-debug.log")

    private fun dbg(msg: String) {
        try { debugLog.appendText("$msg\n") } catch (e: Throwable) { /* ignore */ }
    }

    /** Jump to the terminal tab running this session (delegated to the shared jumper). */
    private fun focus(s: Session) {
        TerminalJump.jumpToTty(project, s.tty, bringToFront = false)
    }
}

private class SessionRenderer : DefaultListCellRenderer() {
    override fun getListCellRendererComponent(
        list: JList<*>?, value: Any?, index: Int, selected: Boolean, focused: Boolean
    ): Component {
        val c = super.getListCellRendererComponent(list, value, index, selected, focused) as JLabel
        val s = value as? Session ?: return c
        val glyph = when (s.state) {
            "needs" -> "🔴"; "yourturn" -> "🟡"; "working" -> "🟢"
            "done" -> "✅"; else -> "⚪"
        }
        val name = if (s.tty.isEmpty()) "${s.topic} (ide)" else s.topic
        val state = when (s.state) {
            "needs" -> "needs you"; "yourturn" -> "your turn"; "working" -> "working"
            "done" -> "done · safe to close"; else -> "idle"
        }
        val extra = if ((s.state == "needs" || s.state == "yourturn") && s.message.isNotEmpty())
            "  —  ${s.message.take(48)}" else ""
        c.text = "$glyph  $name   —   $state ${age(s.updatedAt)}$extra"
        if (!selected) c.foreground = when (s.state) {
            "needs" -> Color(0xE0, 0x4C, 0x4C); "yourturn" -> Color(0xD0, 0xA0, 0x10)
            "working" -> Color(0x3C, 0xA0, 0x4C); "done" -> Color(0x90, 0x90, 0x90); else -> Color(0x88, 0x88, 0x88)
        }
        return c
    }

    private fun age(ts: Double): String {
        if (ts <= 0) return ""
        val s = (System.currentTimeMillis() / 1000.0 - ts).toInt()
        return when {
            s < 60 -> "${s}s"; s < 3600 -> "${s / 60}m"; else -> "${s / 3600}h"
        }
    }
}
