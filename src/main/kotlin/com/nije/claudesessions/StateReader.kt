package com.nije.claudesessions

import com.google.gson.JsonParser
import java.io.File

data class Session(
    val sessionId: String,
    val state: String,
    val topic: String,
    val cwd: String,
    val pid: Long,
    val tty: String,
    val message: String,
    val updatedAt: Double,
)

object StateReader {
    private val stateDir = File(System.getProperty("user.home"), ".claude/session-status/state")

    private val order = mapOf("needs" to 0, "yourturn" to 1, "working" to 2, "idle" to 3, "done" to 4)

    private fun alive(pid: Long): Boolean {
        if (pid <= 0) return true
        return try {
            ProcessHandle.of(pid).isPresent
        } catch (e: Exception) {
            true
        }
    }

    fun load(): List<Session> {
        val files = stateDir.listFiles { f -> f.name.endsWith(".json") } ?: return emptyList()
        val out = ArrayList<Session>()
        for (f in files) {
            try {
                val o = JsonParser.parseString(f.readText()).asJsonObject
                fun str(k: String) = if (o.has(k) && !o.get(k).isJsonNull) o.get(k).asString else ""
                fun lng(k: String) = if (o.has(k) && !o.get(k).isJsonNull) o.get(k).asLong else 0L
                fun dbl(k: String) = if (o.has(k) && !o.get(k).isJsonNull) o.get(k).asDouble else 0.0
                val pid = if (o.has("pid")) lng("pid") else lng("ppid")
                if (!alive(pid)) {
                    f.delete()
                    continue
                }
                out.add(
                    Session(
                        sessionId = str("session_id"),
                        state = str("state"),
                        topic = str("topic"),
                        cwd = str("cwd"),
                        pid = pid,
                        tty = str("tty"),
                        message = str("message"),
                        updatedAt = dbl("updated_at"),
                    )
                )
            } catch (e: Exception) {
                // skip unreadable/partial file
            }
        }
        return out.sortedWith(compareBy({ order[it.state] ?: 9 }, { -it.updatedAt }))
    }
}
