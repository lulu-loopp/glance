package dev.weiyi.glance.remote

import android.content.Context
import android.os.Build
import org.json.JSONObject
import java.io.IOException
import java.io.InputStream
import java.net.URL
import java.net.URLEncoder
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLContext

/**
 * Sending to a paired PC as LocalSend sends (see `docs/nearby.md`): the PC's receiver is found over what it is paired
 * by, pinned by its certificate's fingerprint, and offered files, or words as a message, by this phone's own
 * certificate (see [Own]). New photos sent by themselves ([AutoSend]) and what its user hands over ([Hand]) go so.
 */
object Nearby {
    /**
     * One thing to send: its id in the sending, the name and type the PC is told, how large it is, and how its bytes
     * are read. Or words, sent as a message (nothing of them read: they are told whole).
     */
    class Thing(val id: String, val name: String, val type: String, val size: Long, val words: String? = null, val read: (() -> InputStream)? = null) {
        companion object {
            /** Words sent as a message: LocalSend's text, one text "file" whose words are its preview. */
            fun words(id: String, words: String) = Thing(id, "$id.txt", "text/plain", words.toByteArray().size.toLong(), words)
        }
    }

    /** Where a PC receives, as it says (see the helper's `told_to_phones`): reached at [pc], which may have moved. */
    class Receiving(val pc: Pc, private val said: JSONObject) {
        val on get() = said.optBoolean("on") && port in 1..65535 && fingerprint.length == 64
        val port get() = said.optInt("port")
        val fingerprint: String get() = said.optString("fingerprint")
        /** The most one file may be, all of one sending (none said: no more than any), and how many files one holds. */
        val mostFile get() = said.optLong("most_file")
        val mostSending get() = if (said.isNull("most_sending")) Long.MAX_VALUE else said.optLong("most_sending")
        val mostFiles get() = said.optInt("most_files").coerceAtLeast(1)
    }

    /**
     * Where [pc] receives, as it says over the signed channel: looked for on the network where it is not at its address
     * (and kept where it is found). None where it is not reached, or says nothing of it (an older Glance).
     */
    fun reach(context: Context, pc: Pc): Receiving? {
        try {
            return Receiving(pc, pc.said("/v1/nearby"))
        } catch (failed: Failure.Failed) {
            return null
        } catch (failure: Failure) {
            // Not answered, or answered by one that does not know this phone's key (another machine at its old
            // address): the PC itself is looked for.
        }
        val host = pc.find()?.takeIf { it != pc.host } ?: return null
        Pcs.move(context, pc.key, host)
        val moved = pc.copy(host = host)
        return try { Receiving(moved, moved.said("/v1/nearby")) } catch (failure: Failure) { null }
    }

    /** How a sending went. */
    enum class Ended {
        /** All of it taken (words told, files uploaded, or nothing of it wanted). */
        Sent,
        /** Turned down by the PC's user (or a PIN asked for, which Glance never asks). */
        Refused,
        /** The PC is receiving another sending: it may be tried again in a moment. */
        Busy,
        /** More than the PC has room for, or a file larger than it takes. */
        NoRoom,
        /** Not reached, or broken off on the way: what was taken of it stays taken. */
        Broken,
        /** Stopped by this phone. */
        Stopped,
    }

    /**
     * Offers [things] to [at] as one sending, from this phone (calling itself [Own.alias]), and sends each the PC wants:
     * [took] told of each as the PC has it (a thing it does not want is told so too: it has it), [moved] of the bytes of
     * each as they go. [stopped] is asked as it goes: once it is true, the sending is cancelled.
     */
    fun send(context: Context, at: Receiving, things: List<Thing>, stopped: () -> Boolean, moved: (Thing, Long) -> Unit = { _, _ -> }, took: (Thing) -> Unit): Ended {
        val tls = SSLContext.getInstance("TLS").apply { init(arrayOf(Own.Presenting()), arrayOf(Pinned(at.fingerprint)), null) }
        fun open(path: String): HttpsURLConnection = (URL("https://${at.pc.host}:${at.port}/api/localsend/v2$path").openConnection() as HttpsURLConnection).apply {
            sslSocketFactory = tls.socketFactory
            // (The certificate is pinned: the name it is reached by says nothing.)
            setHostnameVerifier { _, _ -> true }
            connectTimeout = 4000
            readTimeout = 60_000
            setRequestProperty("Connection", "close")
        }
        // The offer: as one sending.
        val offer = JSONObject()
            .put("info", JSONObject().put("alias", Own.alias(context)).put("version", "2.1").put("deviceModel", Build.MODEL).put("deviceType", "mobile")
                .put("fingerprint", Own.fingerprint()).put("port", 53317).put("protocol", "https").put("download", false))
            .put("files", JSONObject().apply {
                for (thing in things) put(thing.id, JSONObject().put("id", thing.id).put("fileName", thing.name).put("size", thing.size).put("fileType", thing.type).apply { thing.words?.let { put("preview", it) } })
            })
        if (stopped()) return Ended.Stopped
        val (status, answer) = try {
            val asking = open("/prepare-upload")
            try {
                asking.requestMethod = "POST"
                asking.doOutput = true
                asking.setRequestProperty("Content-Type", "application/json")
                asking.outputStream.use { it.write(offer.toString().toByteArray()) }
                val status = asking.responseCode
                status to ((if (status < 400) asking.inputStream else asking.errorStream)?.bufferedReader()?.use { it.readText() } ?: "")
            } finally {
                asking.disconnect()
            }
        } catch (failed: IOException) {
            return Ended.Broken
        }
        val said = runCatching { JSONObject(answer) }.getOrNull()
        when {
            status == 200 && said != null -> {}
            // Nothing wanted of them (words are told whole with the offer).
            status == 204 -> return Ended.Sent.also { things.forEach(took) }
            status == 401 || (status == 403 && said?.optString("message") in REFUSALS) -> return Ended.Refused
            status == 403 -> return Ended.NoRoom
            status == 409 -> return Ended.Busy
            else -> return Ended.Broken
        }
        val session = said!!.optString("sessionId")
        val tokens = said.optJSONObject("files") ?: JSONObject()
        fun cancel() = runCatching { open("/cancel?sessionId=${URLEncoder.encode(session, "UTF-8")}").apply { requestMethod = "POST" }.let { it.responseCode; it.disconnect() } }
        for (thing in things) {
            val token = tokens.optString(thing.id)
            // (One the PC does not want is not sent: it has it.)
            if (token.isEmpty()) {
                took(thing)
                continue
            }
            val query = listOf("sessionId" to session, "fileId" to thing.id, "token" to token).joinToString("&") { (name, value) -> "$name=${URLEncoder.encode(value, "UTF-8")}" }
            val ok = !stopped() && try {
                val upload = open("/upload?$query")
                try {
                    upload.requestMethod = "POST"
                    upload.doOutput = true
                    upload.setFixedLengthStreamingMode(thing.size)
                    upload.setRequestProperty("Content-Type", "application/octet-stream")
                    val from = thing.read?.invoke() ?: throw IOException("nothing to read")
                    from.use { input ->
                        upload.outputStream.use { output ->
                            val some = ByteArray(64 * 1024)
                            var done = 0L
                            while (true) {
                                if (stopped()) throw IOException("stopped")
                                val read = input.read(some)
                                if (read < 0) break
                                output.write(some, 0, read)
                                done += read
                                moved(thing, done)
                            }
                        }
                    }
                    upload.responseCode == 200
                } finally {
                    upload.disconnect()
                }
            } catch (failed: IOException) {
                false
            } catch (gone: SecurityException) {
                // (No longer readable: what it was read by was taken back.)
                false
            }
            if (!ok) {
                // The PC keeps what it took of this sending.
                cancel()
                return if (stopped()) Ended.Stopped else Ended.Broken
            }
            took(thing)
        }
        return Ended.Sent
    }

    /** What a PC's user said in refusing a sending (see the helper's `prepare`). */
    private val REFUSALS = setOf("refused", "not taken from devices not trusted")
}
