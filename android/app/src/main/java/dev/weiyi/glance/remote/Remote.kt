package dev.weiyi.glance.remote

import android.content.Context
import android.net.Uri
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.security.SecureRandom
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/** The PC paired with: where it is, the key shared with it, and its name. */
data class Pc(val host: String, val port: Int, val key: String, val name: String) {
    companion object {
        /** A pairing link, as the PC's QR code holds it: `glance://pair?h=…&p=…&k=…&n=…`. */
        fun fromLink(link: String): Pc? {
            val uri = runCatching { Uri.parse(link.trim()) }.getOrNull() ?: return null
            if (uri.scheme != "glance" || uri.host != "pair") return null
            val host = uri.getQueryParameter("h")?.takeIf { it.isNotBlank() } ?: return null
            val port = uri.getQueryParameter("p")?.toIntOrNull()?.takeIf { it in 1..65535 } ?: return null
            val key = uri.getQueryParameter("k")?.takeIf { it.length == 64 && it.all { c -> c.isDigit() || c in 'a'..'f' } } ?: return null
            return Pc(host, port, key, uri.getQueryParameter("n") ?: host)
        }
    }

    fun toJson(): JSONObject = JSONObject().put("host", host).put("port", port).put("key", key).put("name", name)
}

/**
 * This phone, as it tells each PC: a number it gave itself once (a PC's
 * key is this phone's alone from its first use) and what it is called.
 */
object Phone {
    var id = ""
        private set
    val name: String get() = listOf(android.os.Build.MANUFACTURER, android.os.Build.MODEL).filter { it.isNotBlank() }.joinToString(" ")

    /** Reads the number kept, or makes and keeps one. */
    fun know(context: Context) {
        if (id.isNotEmpty()) return
        val kept = context.getSharedPreferences("phone", Context.MODE_PRIVATE)
        id = kept.getString("id", null) ?: java.util.UUID.randomUUID().toString().also { kept.edit().putString("id", it).apply() }
    }
}

/** The PCs paired with, and the one being looked at: kept in the app's own storage. */
object Pcs {
    private fun kept(context: Context) = context.getSharedPreferences("pcs", Context.MODE_PRIVATE).also { Phone.know(context) }

    fun all(context: Context): List<Pc> {
        val list = runCatching { JSONArray(kept(context).getString("list", "[]")) }.getOrElse { JSONArray() }
        return (0 until list.length()).mapNotNull { at ->
            val one = list.optJSONObject(at) ?: return@mapNotNull null
            Pc(one.optString("host"), one.optInt("port"), one.optString("key"), one.optString("name")).takeIf { it.host.isNotBlank() && it.key.length == 64 }
        }
    }

    private fun keep(context: Context, list: List<Pc>, chosen: Int) =
        kept(context).edit().putString("list", JSONArray(list.map(Pc::toJson)).toString()).putInt("chosen", chosen).apply()

    /** The one being looked at, as its place among them (0 when there is none). */
    fun chosen(context: Context): Int = kept(context).getInt("chosen", 0).coerceIn(0, (all(context).size - 1).coerceAtLeast(0))

    fun choose(context: Context, at: Int) = keep(context, all(context), at)

    /** Pairs with [pc] (a PC paired before, by its key, is paired anew in its place) and looks at it. */
    fun add(context: Context, pc: Pc) {
        val list = all(context).toMutableList()
        val at = list.indexOfFirst { it.key == pc.key }
        if (at >= 0) list[at] = pc else list.add(pc)
        keep(context, list, if (at >= 0) at else list.size - 1)
    }

    /** The PC with that key is at [host] now. */
    fun move(context: Context, key: String, host: String) {
        keep(context, all(context).map { if (it.key == key) it.copy(host = host) else it }, chosen(context))
    }

    fun remove(context: Context, at: Int) {
        val list = all(context).toMutableList()
        if (at in list.indices) list.removeAt(at)
        keep(context, list, (at - 1).coerceAtLeast(0))
    }
}

/** One of the PC's monitors: its name, where it is on the desktop, and whether it is on and how bright (percent), each none where it cannot be asked nor changed by itself. */
data class Monitor(val name: String, val x: Int, val y: Int, val width: Int, val height: Int, val on: Boolean?, val brightness: Int?)

/** How the lights are when they are on: their colour (RRGGBB), what they do ("steady", "breathing", "rainbow", "heat"), and how bright they are (percent). */
data class Light(val color: String, val effect: String, val brightness: Int = 100)

/** One of the PC's power plans: what the PC knows it by, its name there, and whether it is the one in use. */
data class Plan(val id: String, val name: String, val on: Boolean)

/** The light under a laptop's keys: how bright it is set to be while the lights are on (none: not chosen yet, and left as it is), of how bright it goes. */
data class Keys(val level: Int?, val most: Int)

/** What is playing on the PC ([title] empty where its program does not say: then only that it sounds). */
data class Media(val title: String, val artist: String, val app: String, val playing: Boolean)

/** One part of the PC: how warm it is (°C) and how busy (%), each where it is known. */
data class Read(val temp: Int?, val load: Int?)

/** The PC's parts as Glance reads them there, and how fast its fans turn. */
data class Reads(val cpu: Read, val gpu: Read, val ram: Read, val disk: Read, val cpuFan: Int?, val caseFan: Int?) {
    fun of(part: Part) = when (part) { Part.Cpu -> cpu; Part.Gpu -> gpu; Part.Ram -> ram; Part.Disk -> disk }
}

enum class Part { Cpu, Gpu, Ram, Disk }

private fun JSONObject.int(name: String) = if (isNull(name)) null else optInt(name)

private fun JSONObject.read(name: String) = optJSONObject(name)?.let { Read(it.int("temp"), it.int("load")) } ?: Read(null, null)

/** The PC as it says it is. */
data class PcState(
    val name: String, val lights: Boolean?, val night: Boolean, val screen: Boolean?, val volume: Int?, val muted: Boolean?, val monitors: List<Monitor> = emptyList(),
    val light: Light = Light("FFFFFF", "steady"), val media: Media? = null, val reads: Reads? = null, val power: List<Plan> = emptyList(), val keys: Keys? = null,
    /** Whether the PC has any lights this can put out and light at all, and whether it can give them an effect and a colour. */
    val lightsThere: Boolean = true, val coloursThere: Boolean = true,
    /**
     * Whether the PC keeps the colours to choose from, for every phone and its own settings (one that does not is
     * older: each phone then keeps its own); and those it keeps, once any were kept.
     */
    val keepsColours: Boolean = false, val colours: List<String>? = null,
    /** A laptop: drawn as one, its lights called what they are (the light under its keys). */
    val laptop: Boolean = false,
) {
    companion object {
        fun from(json: JSONObject) = PcState(
            light = json.optJSONObject("light")?.let { Light(it.optString("color", "FFFFFF"), it.optString("effect", "steady"), it.optInt("brightness", 100)) } ?: Light("FFFFFF", "steady"),
            power = json.optJSONArray("power")?.let { list -> (0 until list.length()).map { at -> list.getJSONObject(at).let { Plan(it.optString("id"), it.optString("name"), it.optBoolean("on")) } } } ?: emptyList(),
            lightsThere = json.optBoolean("lights_there", true),
            coloursThere = json.optBoolean("colours_there", true),
            keepsColours = json.has("colours"),
            laptop = json.optBoolean("laptop"),
            colours = json.optJSONArray("colours")?.let { list -> (0 until list.length()).map { list.optString(it).uppercase() }.filter { it.length == 6 } },
            keys = json.optJSONObject("keys")?.let { Keys(it.int("level"), it.optInt("most", 3)) },
            media = json.optJSONObject("media")?.let { Media(it.optString("title"), it.optString("artist"), it.optString("app"), it.optBoolean("playing")) },
            reads = json.optJSONObject("reads")?.let { Reads(it.read("cpu"), it.read("gpu"), it.read("ram"), it.read("disk"), it.int("cpu_fan"), it.int("case_fan")) },
            name = json.optString("name"),
            lights = if (json.isNull("lights")) null else json.optBoolean("lights"),
            night = json.optBoolean("night"),
            screen = if (json.isNull("screen")) null else json.optBoolean("screen"),
            volume = if (json.isNull("volume")) null else json.optInt("volume"),
            muted = if (json.isNull("muted")) null else json.optBoolean("muted"),
            monitors = json.optJSONArray("monitors")?.let { list ->
                (0 until list.length()).map { at ->
                    val one = list.getJSONObject(at)
                    Monitor(one.optString("name"), one.optInt("x"), one.optInt("y"), one.optInt("width"), one.optInt("height"), if (one.isNull("on")) null else one.optBoolean("on"), one.int("brightness"))
                }
            } ?: emptyList(),
        )
    }
}

/** What went wrong asking the PC. */
sealed class Failure : Exception() {
    /** No answer: the PC is off, asleep, or on another network. */
    object Unreachable : Failure()
    /** The PC does not take this phone's key (paired anew since, perhaps). */
    object Refused : Failure()
    /** The PC could not do it, in its words. */
    class Failed(val why: String) : Failure()
}

private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }

private fun unhex(text: String) = ByteArray(text.length / 2) { text.substring(2 * it, 2 * it + 2).toInt(16).toByte() }

/** The most of an answer that is read: none of the PC's is near it. */
private const val MOST = 256 * 1024

/**
 * What a PC said of itself when it last refused a request as not of now:
 * which run of its helper it is (a request is signed for one), and how far
 * its clock is from this phone's (seconds). By the PC's key.
 */
private val told = java.util.concurrent.ConcurrentHashMap<String, Pair<String, Long>>()

private fun mac(key: ByteArray, parts: List<String>, body: ByteArray): String {
    val mac = Mac.getInstance("HmacSHA256").apply { init(SecretKeySpec(key, "HmacSHA256")) }
    for (part in parts) mac.update("$part\n".toByteArray())
    return hex(mac.doFinal(body))
}

/**
 * Asks the PC for [path] (with [body], a POST; without, a GET), signed with
 * the shared key over what is asked, when, by which phone, for which run of
 * the PC's helper, and a number used once; the answer is signed by the PC
 * in turn, and none that is not is believed. Blocks: call it off the main
 * thread. A long wait is allowed: the PC may be starting its lighting
 * program.
 */
fun Pc.ask(path: String, body: JSONObject? = null): PcState = PcState.from(said(path, body))

/** What the PC answers [path] (see [ask]), as it says it. */
fun Pc.said(path: String, body: JSONObject? = null): JSONObject = said(path, body, again = true)

private fun Pc.said(path: String, body: JSONObject?, again: Boolean): JSONObject {
    val secret = unhex(key)
    val method = if (body == null) "GET" else "POST"
    val sent = body?.toString()?.toByteArray() ?: ByteArray(0)
    val (session, ahead) = told[key] ?: ("" to 0L)
    val time = (System.currentTimeMillis() / 1000 + ahead).toString()
    val nonce = hex(ByteArray(8).also { SecureRandom().nextBytes(it) })
    val name = java.net.URLEncoder.encode(Phone.name, "UTF-8").replace("+", "%20")
    val sign = mac(secret, listOf(method, path, time, nonce, Phone.id, name, session), sent)
    val connection = try {
        (URL("http://$host:$port$path").openConnection() as HttpURLConnection).apply {
            requestMethod = method
            connectTimeout = 2500
            readTimeout = 60000
            setRequestProperty("X-Glance-Time", time)
            setRequestProperty("X-Glance-Nonce", nonce)
            setRequestProperty("X-Glance-Sign", sign)
            setRequestProperty("X-Glance-Session", session)
            // Which phone this is: the PC keeps a key to the phone that first used it.
            setRequestProperty("X-Glance-Phone", Phone.id)
            setRequestProperty("X-Glance-Name", name)
            if (body != null) {
                doOutput = true
                setRequestProperty("Content-Type", "application/json")
                outputStream.use { it.write(sent) }
            }
        }
    } catch (error: Exception) {
        throw Failure.Unreachable
    }
    try {
        val status = try { connection.responseCode } catch (error: Exception) { throw Failure.Unreachable }
        val answer = try {
            (if (status == 200) connection.inputStream else connection.errorStream)?.use { from ->
                val all = java.io.ByteArrayOutputStream()
                val some = ByteArray(8192)
                while (true) {
                    val read = from.read(some)
                    if (read < 0) break
                    all.write(some, 0, read)
                    if (all.size() > MOST) throw Failure.Unreachable
                }
                all.toByteArray()
            } ?: ByteArray(0)
        } catch (error: java.io.IOException) {
            throw Failure.Unreachable
        }
        // The PC's own, to this asking: signed as only the holder of the key can.
        val signed = java.security.MessageDigest.isEqual(mac(secret, listOf("answer", nonce, status.toString()), answer).toByteArray(), connection.getHeaderField("X-Glance-Sign").orEmpty().toByteArray())
        val json = runCatching { JSONObject(answer.decodeToString()) }.getOrElse { JSONObject() }
        return when {
            // (Refused by one that does not know the key: there is nothing it could sign with.)
            status == 401 && !signed -> throw Failure.Refused
            // Whatever else is not signed is not the PC's.
            !signed -> throw Failure.Unreachable
            status == 200 -> json
            // Not of now, as the PC has it: asked again, for the run it says it is and by its clock.
            status == 401 && json.optString("error") == "stale" && again -> {
                told[key] = json.optString("session") to (json.optLong("time") - System.currentTimeMillis() / 1000)
                said(path, body, again = false)
            }
            status == 401 -> throw Failure.Refused
            else -> throw Failure.Failed(json.optString("error", status.toString()))
        }
    } finally {
        connection.disconnect()
    }
}

fun on(value: Boolean) = JSONObject().put("on", value)

/**
 * Whether this phone is on the network the PC's address is in: one of the
 * phone's own networks holds that address. None where the PC is known by a
 * name rather than an address (nothing is looked up for it).
 */
fun Pc.near(): Boolean? {
    val literal = host.all { it.isDigit() || it == '.' } || host.contains(':')
    if (!literal) return null
    val there = runCatching { java.net.InetAddress.getByName(host).address }.getOrNull() ?: return null
    val mine = runCatching { java.net.NetworkInterface.getNetworkInterfaces().toList() }.getOrDefault(emptyList()).filter { it.isUp && !it.isLoopback }.flatMap { it.interfaceAddresses }
    return mine.any { own ->
        val here = own.address.address
        val bits = own.networkPrefixLength.toInt()
        here.size == there.size && bits > 0 && (0 until bits).all { bit -> (here[bit / 8].toInt() xor there[bit / 8].toInt()) and (0x80 ushr (bit % 8)) == 0 }
    }
}

/**
 * Looks for the PC on the network the phone is on now (its address may
 * have changed since it was paired): everyone there is asked for the PC
 * that holds this key, by the key's name; the PC proves it holds it. Its
 * address, or none if it does not answer. Blocks for a second at most.
 */
fun Pc.find(): String? {
    val secret = unhex(key)
    val id = hex(java.security.MessageDigest.getInstance("SHA-256").digest(secret).copyOf(8))
    val nonce = hex(ByteArray(8).also { SecureRandom().nextBytes(it) })
    val proof = hex(Mac.getInstance("HmacSHA256").apply { init(SecretKeySpec(secret, "HmacSHA256")) }.doFinal("here\n$nonce".toByteArray()))
    return runCatching {
        java.net.DatagramSocket().use { socket ->
            socket.broadcast = true
            socket.soTimeout = 350
            val asked = "glance-find $id $nonce".toByteArray()
            // To everyone on each network this phone is on.
            val everyone = java.net.NetworkInterface.getNetworkInterfaces().toList().filter { it.isUp && !it.isLoopback }.flatMap { it.interfaceAddresses }.mapNotNull { it.broadcast } + java.net.InetAddress.getByName("255.255.255.255")
            for (address in everyone.distinct()) runCatching { socket.send(java.net.DatagramPacket(asked, asked.size, address, port)) }
            val answer = java.net.DatagramPacket(ByteArray(256), 256)
            val until = System.currentTimeMillis() + 1000
            while (System.currentTimeMillis() < until) {
                try {
                    socket.receive(answer)
                } catch (late: java.net.SocketTimeoutException) {
                    continue
                }
                if (String(answer.data, 0, answer.length) == "glance-here $nonce $proof") return@use answer.address.hostAddress
            }
            null
        }
    }.getOrNull()
}

/** What the main page shows, and the colours to light the PC in: this phone's own, kept in the app's storage. */
object Look {
    /** The parts of the main page that may be left out, by the names they are kept under. */
    val PARTS = listOf("rig", "reads", "power", "lights", "monitors", "sound", "sleep")

    private fun kept(context: Context) = context.getSharedPreferences("look", Context.MODE_PRIVATE)

    /** Which parts are shown (all, till one is turned off). */
    fun shown(context: Context): Map<String, Boolean> = PARTS.associateWith { kept(context).getBoolean("show_$it", true) }

    fun show(context: Context, part: String, on: Boolean) = kept(context).edit().putBoolean("show_$part", on).apply()

    /** The colours a phone starts with. */
    private val GIVEN = listOf("FF7B4A", "FFFFFF", "70B3FF", "59C98A", "C68CFF", "FF5E7E")

    private fun list(text: String?) = text.orEmpty().split(',').filter { it.length == 6 }

    /**
     * The colours to choose from (RRGGBB), in their order: those given,
     * till any is taken away, and those made by hand after them. (Before
     * the given ones could be taken away, only those made by hand were
     * kept, as "colours".)
     */
    fun colours(context: Context): List<String> = kept(context).getString("palette", null)?.let(::list) ?: (GIVEN + list(kept(context).getString("colours", "")))

    fun keep(context: Context, colours: List<String>) = kept(context).edit().putString("palette", colours.joinToString(",")).apply()

    /** The colours made on this phone: those it keeps that it did not start with. */
    fun made(context: Context): List<String> = colours(context).filter { it !in GIVEN }

    /**
     * Whether this phone's colours have been given to the PC with key [pc], which keeps them for every phone: given
     * once, the first time it is reached after it began keeping them.
     */
    fun given(context: Context, pc: String) = kept(context).getBoolean("given_${pc.take(8)}", false)

    fun give(context: Context, pc: String) = kept(context).edit().putBoolean("given_${pc.take(8)}", true).apply()
}

/**
 * Whether there is a newer app than this one, as Glance's site says: asked as the app is opened, every few hours
 * at most (once a day, one released after the day's asking went untold till the next). (The
 * app is not from a store: nothing else would tell of a newer one.) The site counts the asking, by the day and the
 * network it comes from, and is told nothing of the phone but this app's version.
 */
object Newer {
    private const val SITE = "https://glancepc.com"

    /** Where the newest app is got. */
    const val PAGE = "$SITE/app/"

    private fun kept(context: Context) = context.getSharedPreferences("newer", Context.MODE_PRIVATE)

    private fun parts(version: String) = version.split('.').map { it.toIntOrNull() ?: 0 }

    /** Whether version [one] is later than [other]. */
    fun later(one: String, other: String): Boolean {
        val (a, b) = parts(one) to parts(other)
        for (at in 0 until maxOf(a.size, b.size)) {
            val (x, y) = (a.getOrElse(at) { 0 }) to (b.getOrElse(at) { 0 })
            if (x != y) return x > y
        }
        return false
    }

    /** The newer version last heard of, if this app is still older than it. */
    fun known(context: Context): String? = kept(context).getString("version", null)?.takeIf { later(it, BuildConfig.VERSION_NAME) }

    /** Where the newer app (see [known]) is downloaded from in the app, if the site said (see `Update`). */
    fun apk(context: Context): String? = kept(context).getString("apk", null)?.takeIf { known(context) != null && Update.allowed(it) }

    /** How long after the site was asked it is not asked again. */
    private const val ASK_EVERY_MS = 3 * 60 * 60 * 1000L

    /** Asks the site, unless it was asked lately (asked [now], whenever it was): not from the main thread. Whether it
     *  answered (not asked: as if it had). */
    fun ask(context: Context, now: Boolean = false): Boolean {
        val at = System.currentTimeMillis()
        // (Kept as a moment since 0.3.11; a day's number, as it was kept before, is long past.)
        if (!now && at - kept(context).getLong("asked_at", 0) < ASK_EVERY_MS) return true
        val asking = (java.net.URL("$SITE/api/app?v=${BuildConfig.VERSION_NAME}").openConnection() as java.net.HttpURLConnection).apply {
            connectTimeout = 8000
            readTimeout = 8000
            setRequestProperty("User-Agent", "GlanceApp/${BuildConfig.VERSION_NAME}")
        }
        try {
            val answer = JSONObject(asking.inputStream.bufferedReader().use { it.readText() })
            val said = answer.optString("version")
            // (And where the app of that version is: from a site that does not say, it is got from the site's page.)
            val apk = answer.optString("apk").takeIf(Update::allowed)
            if (!Regex("[0-9]+(\\.[0-9]+){1,3}").matches(said)) return false
            kept(context).edit().putString("version", said).putString("apk", apk).putLong("asked_at", at).apply()
            return true
        } catch (_: Exception) {
            // Not reached, or not answered as expected: asked again the next time the app is opened.
            return false
        } finally {
            asking.disconnect()
        }
    }
}
