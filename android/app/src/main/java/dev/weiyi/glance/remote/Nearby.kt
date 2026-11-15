package dev.weiyi.glance.remote

import android.content.Context
import android.os.Build
import org.json.JSONObject
import java.io.IOException
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.Timer
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLContext
import kotlin.concurrent.schedule

/**
 * Sending as LocalSend sends (see `docs/nearby.md`): to a paired PC, found over what it is paired by, or to any device
 * found nearby ([Found]), each pinned by its certificate's fingerprint and offered files, or words as a message, by
 * this phone's own certificate (see [Own]). New photos sent by themselves ([AutoSend]) and what its user hands over
 * ([Hand]) go so.
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

    /**
     * Where a device receives: its address and port, over HTTPS (pinned to [fingerprint]) or plain HTTP, and whether it
     * is a paired Glance PC (whose turning down is told apart from its want of room).
     */
    class At(val host: String, val port: Int, val https: Boolean, val fingerprint: String, val glance: Boolean)

    /** Where a PC receives, as it says (see the helper's `told_to_phones`): reached at [pc], which may have moved. */
    class Receiving(val pc: Pc, private val said: JSONObject) {
        val on get() = said.optBoolean("on") && port in 1..65535 && fingerprint.length == 64
        val port get() = said.optInt("port")
        val fingerprint: String get() = if (said.isNull("fingerprint")) "" else said.optString("fingerprint")
        /** The most one file may be, all of one sending (none said: no more than any), and how many files one holds. */
        val mostFile get() = said.optLong("most_file")
        val mostSending get() = if (said.isNull("most_sending")) Long.MAX_VALUE else said.optLong("most_sending")
        val mostFiles get() = said.optInt("most_files").coerceAtLeast(1)
        val at get() = At(pc.host, port, true, fingerprint, glance = true)
    }

    /**
     * Where [pc] receives, as it says over the signed channel: looked for on the network where it is not at its address
     * (and kept where it is found). None where it is not reached, or says nothing of it (an older Glance). The
     * fingerprint it says is kept: what it sends is taken by that certificate.
     */
    fun reach(context: Context, pc: Pc): Receiving? {
        val said = try {
            Receiving(pc, pc.said("/v1/nearby"))
        } catch (failed: Failure.Failed) {
            return null
        } catch (failure: Failure) {
            // Not answered, or answered by one that does not know this phone's key (another machine at its old
            // address): the PC itself is looked for.
            val host = pc.find()?.takeIf { it != pc.host } ?: return null
            Pcs.move(context, pc.key, host)
            val moved = pc.copy(host = host)
            try { Receiving(moved, moved.said("/v1/nearby")) } catch (failure: Failure) { return null }
        }
        if (said.fingerprint.length == 64) Pcs.know(context, pc.key, said.fingerprint.uppercase())
        return said
    }

    /** How a sending went. */
    enum class Ended {
        /** All of it taken (words told, files uploaded, or nothing of it wanted). */
        Sent,
        /** Turned down by the device's user. */
        Refused,
        /** The device asks for a PIN (the one given, if any, was not right). */
        Pin,
        /** The device is receiving another sending: it may be tried again in a moment. */
        Busy,
        /** More than the PC has room for, or a file larger than it takes. */
        NoRoom,
        /** Not reached, or broken off on the way: what was taken of it stays taken. */
        Broken,
        /** Stopped by this phone. */
        Stopped,
    }

    /**
     * What stops a sending from elsewhere: once [stop] is called, what it waits on is given up (the connection it waits
     * on ended) and it is cancelled.
     */
    open class Stop {
        @Volatile
        var stopped = false
            private set

        @Volatile
        private var open: HttpURLConnection? = null

        fun stop() {
            stopped = true
            open?.disconnect()
        }

        /** [connection] used by [use], ended where this is stopped meanwhile. */
        fun <T> through(connection: HttpURLConnection, use: () -> T): T {
            open = connection
            try {
                if (stopped) throw IOException("stopped")
                return use()
            } finally {
                open = null
                connection.disconnect()
            }
        }
    }

    /** How long a device's user is given to take what is offered, and how long its holding it is not yet told. */
    private const val OFFER_MS = 120_000
    private const val HELD_MS = 1000L

    /**
     * Offers [things] to [at] as one sending, from this phone (calling itself [Own.alias]), and sends each the device
     * wants: [took] told of each as it has it (a thing it does not want is told so too: it has it), [moved] of the
     * bytes of each as they go, [held] once the device held the offer a second unanswered (its user is asked). [pin]
     * goes with the offer where the device asked for one. Once [stop] is stopped, the sending is cancelled.
     */
    fun send(
        context: Context, at: At, things: List<Thing>, stop: Stop, pin: String? = null, held: () -> Unit = {},
        moved: (Thing, Long) -> Unit = { _, _ -> }, took: (Thing) -> Unit,
    ): Ended {
        val tls = if (at.https) SSLContext.getInstance("TLS").apply { init(arrayOf(Own.Presenting()), arrayOf(Pinned(at.fingerprint)), null) } else null
        val host = if (at.host.contains(':')) "[${at.host}]" else at.host
        fun open(path: String, wait: Int = 60_000): HttpURLConnection = (URL("${if (at.https) "https" else "http"}://$host:${at.port}${Lan.API}$path").openConnection() as HttpURLConnection).apply {
            if (this is HttpsURLConnection && tls != null) {
                sslSocketFactory = tls.socketFactory
                // (The certificate is pinned: the name it is reached by says nothing.)
                setHostnameVerifier { _, _ -> true }
            }
            connectTimeout = 4000
            readTimeout = wait
            setRequestProperty("Connection", "close")
        }
        fun cancel(session: String?) = runCatching {
            val connection = open(if (session == null) "/cancel" else "/cancel?sessionId=${Lan.escaped(session)}", 10_000).apply { requestMethod = "POST" }
            try { connection.responseCode } finally { connection.disconnect() }
        }
        // The offer: as one sending.
        val offer = JSONObject()
            .put("info", Receiver.me(context).copy(announce = false).toJson())
            .put("files", JSONObject().apply {
                for (thing in things) put(thing.id, JSONObject().put("id", thing.id).put("fileName", thing.name).put("size", thing.size).put("fileType", thing.type).apply { thing.words?.let { put("preview", it) } })
            })
        if (stop.stopped) return Ended.Stopped
        val holding = Timer(true)
        val (status, answer) = try {
            val asking = open(if (pin == null) "/prepare-upload" else "/prepare-upload?pin=${Lan.escaped(pin)}", OFFER_MS)
            holding.schedule(HELD_MS) { held() }
            stop.through(asking) {
                asking.requestMethod = "POST"
                asking.doOutput = true
                asking.setRequestProperty("Content-Type", "application/json")
                asking.outputStream.use { it.write(offer.toString().toByteArray()) }
                val status = asking.responseCode
                status to ((if (status < 400) asking.inputStream else asking.errorStream)?.bufferedReader()?.use { it.readText() } ?: "")
            }
        } catch (failed: IOException) {
            // Stopped while the device's user is asked (no session yet): told so, which a device takes as its sender's.
            if (stop.stopped) {
                cancel(null)
                return Ended.Stopped
            }
            return Ended.Broken
        } finally {
            holding.cancel()
        }
        val said = runCatching { JSONObject(answer) }.getOrNull()
        when {
            status == 200 && said != null -> {}
            // Nothing wanted of them (words are told whole with the offer).
            status == 204 -> return Ended.Sent.also { things.forEach(took) }
            status == 401 -> return Ended.Pin
            // (A device that is not Glance turns down by 403 alone. A paired PC says why: its user, or its room.)
            status == 403 && (!at.glance || said?.optString("message") in REFUSALS) -> return Ended.Refused
            status == 403 -> return Ended.NoRoom
            status == 409 || status == 429 -> return Ended.Busy
            else -> return Ended.Broken
        }
        val session = said!!.optString("sessionId")
        val tokens = said.optJSONObject("files") ?: JSONObject()
        for (thing in things) {
            val token = tokens.optString(thing.id)
            // (One the device does not want is not sent: it has it.)
            if (token.isEmpty()) {
                took(thing)
                continue
            }
            val query = listOf("sessionId" to session, "fileId" to thing.id, "token" to token).joinToString("&") { (name, value) -> "$name=${Lan.escaped(value)}" }
            val ok = !stop.stopped && try {
                val upload = open("/upload?$query")
                stop.through(upload) {
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
                                if (stop.stopped) throw IOException("stopped")
                                val read = input.read(some)
                                if (read < 0) break
                                output.write(some, 0, read)
                                done += read
                                moved(thing, done)
                            }
                        }
                    }
                    upload.responseCode == 200
                }
            } catch (failed: IOException) {
                false
            } catch (gone: SecurityException) {
                // (No longer readable: what it was read by was taken back.)
                false
            }
            if (!ok) {
                // The device keeps what it took of this sending.
                cancel(session)
                return if (stop.stopped) Ended.Stopped else Ended.Broken
            }
            took(thing)
        }
        return Ended.Sent
    }

    /** What a PC's user said in refusing a sending (see the helper's `prepare`). */
    private val REFUSALS = setOf(Lan.REFUSED, Lan.NOT_TRUSTED)
}
