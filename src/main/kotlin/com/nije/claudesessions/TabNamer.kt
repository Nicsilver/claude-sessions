package com.nije.claudesessions

import com.google.gson.Gson
import com.google.gson.JsonParser
import com.intellij.openapi.project.Project
import org.jetbrains.plugins.terminal.TerminalToolWindowManager
import java.io.File

/** Names IDE terminal tabs after the Claude session they host — what Windows Terminal does
 *  natively from the title escapes IntelliJ's terminal ignores. Reads the recorder's
 *  state/<sid>.json topic per tty and applies it as the tab's display name.
 *
 *  Never clobbers a name the user chose: only tabs still carrying a stock title ("Local",
 *  "Local (2)", the launcher's "claude") or the name this object set previously get renamed.
 *  Uses `topic` rather than `tab_title` because tab_title derives FROM the IDE tab name —
 *  renaming from it would freeze the loop on the first applied value. */
object TabNamer {
    private val stateDir = File(System.getProperty("user.home"), ".claude/session-status/state")
    private val appliedFile =
        File(System.getProperty("user.home"), ".claude/session-status/tab-names-applied.json")

    /** tty → name we last set. Persisted so a restored tab (IDE restart keeps tab names but
     *  wipes memory) is still recognised as ours and keeps following its session. */
    private val applied: HashMap<String, String> = runCatching {
        val o = JsonParser.parseString(appliedFile.readText()).asJsonObject
        HashMap(o.entrySet().associate { it.key to it.value.asString })
    }.getOrElse { HashMap() }

    /** The name this object gave the tab on [tty], or null if the tab's name isn't ours —
     *  lets the publisher flag auto names so the recorder won't echo them back as labels. */
    fun appliedName(tty: String): String? = applied[tty]

    fun apply(project: Project) {
        val mgr = TerminalToolWindowManager.getInstance(project)
        val widgets = runCatching { mgr.terminalWidgets }.getOrNull() ?: return
        if (widgets.isEmpty()) return
        val topics = topicsByTty()
        if (topics.isEmpty()) return
        for (w in widgets) {
            val tty = TerminalJump.ttyOf(w)
            val name = topics[tty]?.let(::shorten) ?: continue
            val content = runCatching { mgr.getContainer(w)?.content }.getOrNull() ?: continue
            val cur = content.displayName?.trim().orEmpty()
            if (cur == name) {
                remember(tty, name)
                continue
            }
            if (isDefaultTab(cur) || cur == applied[tty]) {
                runCatching { content.displayName = name }
                remember(tty, name)
            }
        }
    }

    private fun remember(tty: String, name: String) {
        if (applied.put(tty, name) != name) {
            runCatching { appliedFile.writeText(Gson().toJson(applied)) }
        }
    }

    /** Tab-friendly length: word-boundary cut at [n] chars. */
    private fun shorten(s: String, n: Int = 20): String {
        if (s.length <= n) return s
        val cut = s.take(n)
        val head = cut.substringBeforeLast(' ').ifBlank { cut }
        return head.trimEnd() + "…"
    }

    /** The stock titles a tab has when nobody named it. */
    private fun isDefaultTab(name: String): Boolean {
        val base = name.replace(" ", "")
        return base.isEmpty()
            || base == "Local"
            || (base.startsWith("Local(") && base.endsWith(")"))
            || base.equals("claude", ignoreCase = true)
    }

    /** tty → session topic from the recorder's state files (topic is already length-capped
     *  by the recorder's label heuristics). */
    private fun topicsByTty(): Map<String, String> {
        val files = stateDir.listFiles { f -> f.name.endsWith(".json") } ?: return emptyMap()
        val map = HashMap<String, String>()
        for (f in files) {
            runCatching {
                val o = JsonParser.parseString(f.readText()).asJsonObject
                val tty = o.get("tty")?.takeIf { !it.isJsonNull }?.asString.orEmpty()
                val topic = o.get("topic")?.takeIf { !it.isJsonNull }?.asString.orEmpty().trim()
                if (tty.isNotEmpty() && topic.isNotEmpty()) map[tty] = topic
            }
        }
        return map
    }
}
