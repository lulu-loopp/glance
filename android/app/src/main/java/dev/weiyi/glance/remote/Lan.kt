package dev.weiyi.glance.remote

import org.json.JSONException
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream

/**
 * LocalSend v2 as this phone speaks it, both ways (see `docs/nearby.md`): what devices say of themselves and offer,
 * read as what comes from the network is (checked, bounded, never trusted for a path), where a file that comes is
 * kept, who is taken from without asking, and the little of HTTP/1.1 the protocol needs. Nothing here touches the
 * system: the server ([Serving]), discovery ([Found]) and sending ([Nearby]) use it.
 */
object Lan {
    /** The port devices are found on, and received on where it is free. */
    const val PORT = 53317
    const val GROUP = "224.0.0.167"
    const val API = "/api/localsend/v2"
    const val VERSION = "2.1"

    /** The most an offer's files may be, and the most its JSON may be (a message's words come in it). */
    const val MOST_FILES = 10_000
    const val MOST_OFFER = 8 * 1024 * 1024

    /** The most a device's info may be, as it registers. */
    const val MOST_INFO = 64 * 1024

    /** Why what came is answered as it is: an HTTP status, and a word of why. */
    class Bad(val status: Int, override val message: String) : Exception(message)

    /** What a sending is turned down with by its device's user (see the PC's `prepare`: its sender knows these). */
    const val REFUSED = "refused"
    const val NOT_TRUSTED = "not taken from devices not trusted"

    /** A device as it says it is: in its announcement, as it registers, in its offer, or answering `info`. */
    data class Info(
        val alias: String,
        val deviceModel: String?,
        val deviceType: String?,
        val fingerprint: String,
        /** Where it receives, and how (an answer to `register` leaves these out). */
        val port: Int?,
        val https: Boolean,
        /** Whether it announced itself (and is to be answered). */
        val announce: Boolean = false,
        /** Whether it is a Glance (a PC or this app), as every Glance says of itself: other LocalSend apps do not. */
        val glance: Boolean = false,
    ) {
        /** As this phone tells of itself: with where it receives, and as an announcement where [announce]. */
        fun toJson(): JSONObject = JSONObject()
            .put("alias", alias).put("version", VERSION).put("deviceModel", deviceModel ?: JSONObject.NULL).put("deviceType", deviceType ?: JSONObject.NULL)
            .put("fingerprint", fingerprint).put("port", port ?: JSONObject.NULL).put("protocol", if (https) "https" else "http").put("download", false)
            .apply { if (announce) put("announce", true).put("announcement", true) }
            .apply { if (glance) put("glance", true) }

        companion object {
            /** What [json] says of a device: none where it names none (no fingerprint). */
            fun of(json: JSONObject): Info? {
                val fingerprint = json.text("fingerprint")?.trim()?.takeIf { it.isNotEmpty() && it.length <= 128 } ?: return null
                return Info(
                    alias = shownName(json.text("alias").orEmpty()).ifEmpty { "LocalSend" },
                    deviceModel = json.text("deviceModel")?.let(::shownName)?.takeIf { it.isNotEmpty() },
                    deviceType = json.text("deviceType")?.take(20)?.lowercase(),
                    fingerprint = fingerprint,
                    port = if (json.has("port") && !json.isNull("port")) json.optInt("port").takeIf { it in 1..65535 } else null,
                    https = json.text("protocol") != "http",
                    announce = json.optBoolean("announce") || json.optBoolean("announcement"),
                    glance = json.optBoolean("glance"),
                )
            }

            /** [text], as JSON, of a device: none where it is not JSON naming one. */
            fun of(text: String): Info? = try {
                of(JSONObject(text))
            } catch (bad: JSONException) {
                null
            }
        }
    }

    /** A string [name] holds, none where it holds none (JSON's null, or another kind). */
    private fun JSONObject.text(name: String): String? = if (isNull(name)) null else opt(name) as? String

    /** A name a device gave itself, as it is shown: no control characters, on one line, not too long. */
    fun shownName(name: String): String = name.filter { it >= ' ' && it != '\u007F' }.trim().take(64)

    /** One file offered: its id in the sending, its name as sent (a folder's: its place in it), size and type, and its words where it is a message. */
    class Offered(val id: String, val name: String, val size: Long, val type: String, val sha256: String?, val preview: String?)

    /** A sending offered (`prepare-upload`): who offers it, and its files. */
    class Offer(val info: Info, val files: List<Offered>) {
        /** The words, where it is a message (LocalSend's text: one text "file" whose words are its preview). */
        val words: String? get() = files.singleOrNull()?.takeIf { it.type.startsWith("text/") }?.preview?.takeIf { it.isNotBlank() }

        /** All its files together. */
        val total: Long get() = files.sumOf { it.size }

        companion object {
            /** [text] as an offer: [Bad] (400) where it is none. */
            fun of(text: String): Offer {
                val json = try {
                    JSONObject(text)
                } catch (bad: JSONException) {
                    throw Bad(400, "not a sending")
                }
                val info = json.optJSONObject("info")?.let(Info::of) ?: throw Bad(400, "no info of the sender")
                val files = json.optJSONObject("files") ?: throw Bad(400, "no files")
                if (files.length() > MOST_FILES) throw Bad(400, "too many files")
                var total = 0L
                val offered = files.keys().asSequence().map { key ->
                    val file = files.optJSONObject(key) ?: throw Bad(400, "a file that is not one")
                    if (key.length > 200) throw Bad(400, "an id too long")
                    val size = if (file.opt("size") is Number) file.getLong("size") else throw Bad(400, "a file with no size")
                    if (size < 0) throw Bad(400, "a file with no size")
                    total += size
                    if (total < 0) throw Bad(400, "sizes beyond counting")
                    val name = file.text("fileName").orEmpty()
                    if (name.length > 4096) throw Bad(400, "a name too long")
                    val sha256 = file.text("sha256")?.lowercase()?.takeIf { hash -> hash.length == 64 && hash.all { it in '0'..'9' || it in 'a'..'f' } }
                    Offered(key, name, size, file.text("fileType").orEmpty().take(100), sha256, file.text("preview"))
                }.toList()
                return Offer(info, offered)
            }
        }
    }

    // ---- Where what comes is kept ----

    /** Where on the phone a kind of file goes, under its own "Glance" folder: the gallery's pictures and videos, or downloads. */
    enum class Shelf(val root: String) { Pictures("Pictures"), Movies("Movies"), Download("Download") }

    /** Where a file that came is kept: on which shelf, in which folders under its "Glance" (a folder's inner places), named what, of what type. */
    class Place(val shelf: Shelf, val folders: List<String>, val name: String, val type: String) {
        /** Its folder under the storage's root, as MediaStore takes it ("Download/Glance/inner/"). */
        val relative: String get() = "${shelf.root}/Glance/" + folders.joinToString("") { "$it/" }
    }

    /** The most a part of a path may be (bytes, as a file system counts them): ext4 and FAT take 255. */
    private const val MOST_PART = 200

    /** [name] as one part of a path: what no file system takes put as "_", no control characters, not ending in spaces or dots, not too long (its extension kept). */
    fun cleanPart(name: String): String {
        val plain = name.map { if (it < ' ' || it == '\u007F') ' ' else if (it in "\\/:*?\"<>|") '_' else it }.joinToString("").trim().trimEnd('.', ' ')
        if (plain.toByteArray().size <= MOST_PART) return plain
        val dot = plain.lastIndexOf('.')
        val extension = if (dot > 0 && plain.length - dot <= 16) plain.substring(dot) else ""
        var stem = plain.removeSuffix(extension)
        while ((stem + extension).toByteArray().size > MOST_PART) stem = stem.dropLast(1)
        return stem.trimEnd('.', ' ') + extension
    }

    /**
     * The parts of the path a file was sent by ("folder/inner/name", as LocalSend names a folder's files), each made a
     * plain name ([cleanPart]), and a part that names no folder of its own ("", ".", "..") left out: nothing reaches out
     * of where it is put. The last is the file's name ("file" where nothing of one is left).
     */
    fun parts(sent: String): List<String> {
        val parts = sent.split('/', '\\').map(::cleanPart).filter { it.isNotEmpty() && it != "." && it != ".." }
        return parts.ifEmpty { listOf("file") }
    }

    /**
     * Where a file sent as [sent] is kept: in a folder of its own where it came in one (its inner places kept), else
     * by what it is, as its extension says ([typeOf]): a picture with the gallery's pictures, a video with its videos,
     * anything else with the downloads.
     */
    fun place(sent: String, typeOf: (String) -> String?): Place {
        val parts = parts(sent)
        val name = parts.last()
        val extension = name.substringAfterLast('.', "").lowercase()
        val type = typeOf(extension)?.takeIf { extension.isNotEmpty() } ?: "application/octet-stream"
        val folders = parts.dropLast(1)
        val shelf = when {
            folders.isNotEmpty() -> Shelf.Download
            type.startsWith("image/") -> Shelf.Pictures
            type.startsWith("video/") -> Shelf.Movies
            else -> Shelf.Download
        }
        return Place(shelf, folders, name, type)
    }

    // ---- Who is taken from ----

    /** What is done with a sending from a device not paired nor trusted: asked about each time, or refused. */
    enum class Others { Ask, Refuse }

    sealed interface Decision {
        /** Taken without asking. */
        data object Take : Decision
        /** Its user asked, offered to trust the device too only where it proved its certificate. */
        data class Ask(val canTrust: Boolean) : Decision
        data object Refuse : Decision
    }

    /**
     * Whether a sending is taken from a device that presented the certificate of [verified] (none: it presented none,
     * and the fingerprint it says is only said): a paired PC ([paired], their fingerprints as they told them over what
     * they are paired by) and a device trusted on this phone ([trusted]) are taken from without asking, only by the
     * certificate they presented. Anyone else as [others] says.
     */
    fun decide(verified: String?, paired: Set<String>, trusted: Set<String>, others: Others): Decision {
        val proved = verified?.uppercase()
        if (proved != null && (proved in paired.map { it.uppercase() } || proved in trusted.map { it.uppercase() })) return Decision.Take
        return when (others) {
            Others.Refuse -> Decision.Refuse
            Others.Ask -> Decision.Ask(canTrust = proved != null)
        }
    }

    // ---- A pairing link ----

    /**
     * A PC to pair with, as its pairing code holds it: the remote's page with the key after its `#`
     * (`http://<address>:<port>/#k=<key>&n=<name>`, one code for this app and a phone's camera), or, from an older PC,
     * `glance://pair?h=<address>&p=<port>&k=<key>&n=<name>`. None where [link] is neither.
     */
    fun pairing(link: String): Pc? {
        val text = link.trim()
        val (host, port, said) = when {
            text.startsWith("glance://pair?", ignoreCase = true) -> {
                val said = fields(text.substringAfter('?').substringBefore('#'))
                Triple(said["h"].orEmpty(), said["p"]?.toIntOrNull(), said)
            }
            text.startsWith("http://", ignoreCase = true) || text.startsWith("https://", ignoreCase = true) -> {
                val authority = text.substringAfter("://").substringBefore('#').substringBefore('/').substringBefore('?')
                if (!text.contains('#')) return null
                // (An address of IPv6 in brackets, its port after them.)
                val (host, port) = if (authority.startsWith('[')) {
                    authority.substringAfter('[').substringBefore(']') to authority.substringAfter("]", "").removePrefix(":").toIntOrNull()
                } else {
                    authority.substringBefore(':') to authority.substringAfter(':', "").toIntOrNull()
                }
                Triple(host, port, fields(text.substringAfter('#')))
            }
            else -> return null
        }
        if (host.isBlank() || host.any { it.isWhitespace() || it == '/' || it == '@' }) return null
        val at = port?.takeIf { it in 1..65535 } ?: return null
        val key = said["k"]?.lowercase()?.takeIf { it.length == 64 && it.all { c -> c.isDigit() || c in 'a'..'f' } } ?: return null
        return Pc(host, at, key, said["n"]?.let(::shownName)?.takeIf { it.isNotEmpty() } ?: host)
    }

    /** The fields of a query (or of what follows a `#` as one), each unescaped. */
    private fun fields(query: String): Map<String, String> = query.split('&').filter { it.isNotEmpty() }.associate { field ->
        unescaped(field.substringBefore('=')) to unescaped(field.substringAfter('=', ""))
    }

    /** `%XX` and `+` in a URL's part, as what they stand for (bytes of UTF-8). */
    fun unescaped(text: String): String {
        val out = ByteArrayOutputStream()
        val bytes = text.toByteArray()
        var at = 0
        while (at < bytes.size) {
            val byte = bytes[at]
            val high = if (byte == '%'.code.toByte() && at + 2 < bytes.size) Character.digit(bytes[at + 1].toInt(), 16) else -1
            val low = if (high >= 0) Character.digit(bytes[at + 2].toInt(), 16) else -1
            when {
                high >= 0 && low >= 0 -> {
                    out.write(high * 16 + low)
                    at += 3
                    continue
                }
                byte == '+'.code.toByte() -> out.write(' '.code)
                else -> out.write(byte.toInt())
            }
            at++
        }
        return out.toByteArray().decodeToString()
    }

    /** What may not stand in a URL's query as it is, `%`-escaped. */
    fun escaped(text: String): String = text.toByteArray().joinToString("") { byte ->
        val char = (byte.toInt() and 0xFF).toChar()
        if (char.isLetterOrDigit() && char.code < 128 || char in "-_.~") char.toString() else "%%%02X".format(byte.toInt() and 0xFF)
    }

    // ---- HTTP ----

    /** One request as it came: its method, its path (without its query, unescaped), its query, its headers (by lower-case name), and its body as it comes. */
    class Request(val method: String, val path: String, val query: Map<String, String>, val headers: Map<String, String>, val body: InputStream) {
        /** All of its body, as text: [Bad] (413) where it is more than [most]. */
        fun text(most: Int): String {
            val all = ByteArrayOutputStream()
            val some = ByteArray(8192)
            while (true) {
                val read = body.read(some)
                if (read < 0) break
                all.write(some, 0, read)
                if (all.size() > most) throw Bad(413, "too large")
            }
            return all.toByteArray().decodeToString()
        }
    }

    /** The most a request's head may be. */
    private const val MOST_HEAD = 16 * 1024

    /** The request that comes on [input] (read a byte at a time: give it a buffered stream): none where it closed first. */
    fun request(input: InputStream): Request? {
        val head = ByteArrayOutputStream()
        while (true) {
            val byte = input.read()
            if (byte < 0) {
                if (head.size() == 0) return null
                throw Bad(400, "closed within the head")
            }
            head.write(byte)
            if (head.size() > MOST_HEAD) throw Bad(431, "a head too long")
            val bytes = head.size()
            if (bytes >= 4) {
                val all = head.toByteArray()
                if (all[bytes - 4] == '\r'.code.toByte() && all[bytes - 3] == '\n'.code.toByte() && all[bytes - 2] == '\r'.code.toByte() && all[bytes - 1] == '\n'.code.toByte()) break
            }
        }
        val lines = head.toByteArray().decodeToString().split("\r\n")
        val first = lines[0].split(' ')
        if (first.size < 3) throw Bad(400, "no request")
        val headers = lines.drop(1).filter { it.contains(':') }.associate { it.substringBefore(':').trim().lowercase() to it.substringAfter(':').trim() }
        val target = first[1]
        val body: InputStream = when {
            headers["transfer-encoding"]?.lowercase()?.contains("chunked") == true -> Chunked(input)
            headers.containsKey("content-length") -> Sized(input, headers.getValue("content-length").toLongOrNull()?.takeIf { it >= 0 } ?: throw Bad(400, "a length that is no number"))
            else -> Sized(input, 0)
        }
        return Request(first[0], unescaped(target.substringBefore('?')), fields(target.substringAfter('?', "")), headers, body)
    }

    /** [status] answered on [output], with [json] as its body (or none), the connection ended after it. */
    fun answer(output: OutputStream, status: Int, json: String?) {
        val body = json?.toByteArray() ?: ByteArray(0)
        val head = StringBuilder("HTTP/1.1 $status ${reason(status)}\r\n")
        if (json != null) head.append("Content-Type: application/json\r\n")
        head.append("Content-Length: ${body.size}\r\nConnection: close\r\n\r\n")
        output.write(head.toString().toByteArray())
        output.write(body)
        output.flush()
    }

    private fun reason(status: Int) = when (status) {
        200 -> "OK"
        204 -> "No Content"
        400 -> "Bad Request"
        401 -> "Unauthorized"
        403 -> "Forbidden"
        404 -> "Not Found"
        409 -> "Conflict"
        412 -> "Precondition Failed"
        413 -> "Payload Too Large"
        422 -> "Unprocessable Entity"
        429 -> "Too Many Requests"
        431 -> "Request Header Fields Too Large"
        500 -> "Internal Server Error"
        else -> "Error"
    }

    /** A body of [left] bytes. */
    private class Sized(private val input: InputStream, private var left: Long) : InputStream() {
        override fun read(): Int {
            if (left <= 0) return -1
            val byte = input.read()
            if (byte < 0) throw IOException("closed within the body")
            left--
            return byte
        }

        override fun read(into: ByteArray, at: Int, most: Int): Int {
            if (left <= 0) return -1
            val read = input.read(into, at, minOf(most.toLong(), left).toInt())
            if (read < 0) throw IOException("closed within the body")
            left -= read
            return read
        }
    }

    /** A body that comes in chunks, till the last (of none). */
    private class Chunked(private val input: InputStream) : InputStream() {
        /** What is left of the chunk being read, and whether the last has come. */
        private var left = 0L
        private var over = false

        private fun line(): String {
            val line = StringBuilder()
            while (true) {
                val byte = input.read()
                if (byte < 0) throw IOException("closed within the body")
                if (byte == '\n'.code) return line.toString().trimEnd('\r')
                line.append(byte.toChar())
                if (line.length > 1024) throw Bad(400, "a chunk's line too long")
            }
        }

        /** Whether there is more: the next chunk begun where the last is read. */
        private fun more(): Boolean {
            if (over) return false
            if (left > 0) return true
            val size = line().substringBefore(';').trim().toLongOrNull(16)?.takeIf { it >= 0 } ?: throw Bad(400, "a chunk with no size")
            if (size == 0L) {
                // (What trails it, to its empty line.)
                while (line().isNotEmpty()) continue
                over = true
                return false
            }
            left = size
            return true
        }

        override fun read(): Int {
            if (!more()) return -1
            val byte = input.read()
            if (byte < 0) throw IOException("closed within the body")
            if (--left == 0L) line()
            return byte
        }

        override fun read(into: ByteArray, at: Int, most: Int): Int {
            if (most == 0) return 0
            if (!more()) return -1
            val read = input.read(into, at, minOf(most.toLong(), left).toInt())
            if (read < 0) throw IOException("closed within the body")
            left -= read
            // (Each chunk ends with a line's end of its own.)
            if (left == 0L) line()
            return read
        }
    }
}
