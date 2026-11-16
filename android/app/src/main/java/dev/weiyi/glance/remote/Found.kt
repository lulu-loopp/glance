package dev.weiyi.glance.remote

import android.content.Context
import android.net.wifi.WifiManager
import android.os.SystemClock
import java.io.IOException
import java.net.DatagramPacket
import java.net.HttpURLConnection
import java.net.Inet4Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.MulticastSocket
import java.net.NetworkInterface
import java.net.URL
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLContext
import kotlin.concurrent.thread

/**
 * Finding devices, and being found: LocalSend's multicast group (`224.0.0.167:53317`) on every network this phone is
 * on, the system told to let the group's packets through (a `MulticastLock`) while it is heard. This phone announces
 * itself (as it begins, as its networks change, and as a sheet to send from opens). A device that hears an
 * announcement registers with the announcer over HTTP(S), and this phone does the same with those it hears (or, where
 * that fails, answers in the group, as the protocol allows). [heard] is told of each device as it says it is.
 */
class Found(private val context: Context, private val me: () -> Lan.Info, private val heard: (Lan.Info, String, String?) -> Unit) {
    /** A device found nearby: as it says it is, where it receives, and when it was last heard (elapsed ms). */
    data class Device(val fingerprint: String, val alias: String, val model: String?, val type: String?, val glance: Boolean, val address: String, val port: Int, val https: Boolean, val heard: Long)

    companion object {
        /** A device heard within this long is nearby still (ms). */
        const val FRESH = 180_000L
        /** When an announcement is sent again after the first (the protocol's burst: one may be lost). */
        private val BURST = listOf(100L, 500L, 2000L)
        /** How often this phone's networks are looked at again (a network come, the group is joined there, and this phone announced). */
        private const val NETWORKS_EVERY = 10_000L
        /** A device is registered with at most this often (its announcements come in bursts). */
        private const val REGISTER_EVERY = 5_000L

        /** The networks this phone is on and up, by their IPv4 address (not its loopback, nor a link's own). */
        fun networks(): List<Pair<NetworkInterface, Inet4Address>> = runCatching { NetworkInterface.getNetworkInterfaces().toList() }.getOrDefault(emptyList())
            .filter { runCatching { it.isUp && !it.isLoopback && it.supportsMulticast() }.getOrDefault(false) }
            .flatMap { network -> network.inetAddresses.toList().filterIsInstance<Inet4Address>().filter { !it.isLinkLocalAddress && !it.isLoopbackAddress }.map { network to it } }
    }

    private val group = InetAddress.getByName(Lan.GROUP)
    private val lock: WifiManager.MulticastLock = context.getSystemService(WifiManager::class.java).createMulticastLock("glance-nearby").apply { setReferenceCounted(false) }

    /** What the group is heard on (none where its port is held by another program alone: then this phone only announces), and what announcements go out by. */
    private var hearing: MulticastSocket? = null
    private val out = MulticastSocket().apply { timeToLive = 1 }
    private var joined = listOf<NetworkInterface>()
    private val registered = HashMap<String, Long>()

    @Volatile
    private var running = true

    /** Joins the group on every network, hears it (a thread), watches the networks (another), and announces this phone. */
    fun start() {
        lock.acquire()
        hearing = try {
            MulticastSocket(null).apply {
                reuseAddress = true
                bind(InetSocketAddress(Lan.PORT))
            }
        } catch (held: IOException) {
            null
        }
        hearing?.let { socket -> thread(name = "nearby-hear", isDaemon = true) { hear(socket) } }
        thread(name = "nearby-networks", isDaemon = true) {
            while (running) {
                val now = networks().map { it.first }.distinct()
                val new = now.filter { network -> joined.none { it.name == network.name } }
                joined = now
                for (network in new) runCatching { hearing?.joinGroup(InetSocketAddress(group, Lan.PORT), network) }
                if (new.isNotEmpty()) announce()
                try {
                    Thread.sleep(NETWORKS_EVERY)
                } catch (stopped: InterruptedException) {
                    return@thread
                }
            }
        }
    }

    /** Hears and announces no more, and lets the system hold the group's packets again. */
    fun stop() {
        running = false
        hearing?.close()
        out.close()
        if (lock.isHeld) lock.release()
    }

    /** Sends [what] to the group on every network. */
    private fun toAll(what: ByteArray) = synchronized(out) {
        for ((network, _) in networks()) {
            runCatching {
                out.networkInterface = network
                out.send(DatagramPacket(what, what.size, group, Lan.PORT))
            }
        }
    }

    /** Announces this phone on every network, in the protocol's burst (a thread). */
    fun announce() {
        thread(name = "nearby-announce", isDaemon = true) {
            val what = me().copy(announce = true).toJson().toString().toByteArray()
            var waited = 0L
            for (at in BURST) {
                Thread.sleep(at - waited)
                waited = at
                if (!running) return@thread
                toAll(what)
            }
        }
    }

    /** Hears the group: each device known as it says, and one that announces itself registered with. */
    private fun hear(socket: MulticastSocket) {
        val packet = DatagramPacket(ByteArray(8192), 8192)
        val mine = me().fingerprint
        while (running) {
            try {
                // (All of the buffer again: a packet received shortens it to what came.)
                packet.length = packet.data.size
                socket.receive(packet)
            } catch (closed: IOException) {
                if (!running) return
                continue
            }
            val info = Lan.Info.of(String(packet.data, 0, packet.length)) ?: continue
            if (info.fingerprint.equals(mine, ignoreCase = true)) continue
            val address = packet.address.hostAddress ?: continue
            heard(info, address, null)
            if (!info.announce) continue
            val now = SystemClock.elapsedRealtime()
            val fresh = synchronized(registered) {
                (registered[info.fingerprint]?.let { now - it >= REGISTER_EVERY } ?: true).also { if (it) registered[info.fingerprint] = now }
            }
            if (fresh) thread(name = "nearby-register", isDaemon = true) { register(address, info) }
        }
    }

    /**
     * Tells the device at [address] that announced [info] of this phone (`register`), pinned to the fingerprint it
     * announced and presenting this phone's certificate: what it says back is known, by the certificate it proved.
     * Where it cannot be reached so, this phone answers in the group instead.
     */
    private fun register(address: String, info: Lan.Info) {
        val port = info.port ?: Lan.PORT
        val host = if (address.contains(':')) "[$address]" else address
        val answered = try {
            val connection = (URL("${if (info.https) "https" else "http"}://$host:$port${Lan.API}/register").openConnection() as HttpURLConnection).apply {
                if (this is HttpsURLConnection) {
                    sslSocketFactory = SSLContext.getInstance("TLS").apply { init(arrayOf(Own.Presenting()), arrayOf(Pinned(info.fingerprint)), null) }.socketFactory
                    setHostnameVerifier { _, _ -> true }
                }
                connectTimeout = 3000
                readTimeout = 10_000
                requestMethod = "POST"
                doOutput = true
                setRequestProperty("Content-Type", "application/json")
                setRequestProperty("Connection", "close")
            }
            try {
                connection.outputStream.use { it.write(me().copy(announce = false).toJson().toString().toByteArray()) }
                if (connection.responseCode != 200) throw IOException("register answered ${connection.responseCode}")
                val text = connection.inputStream.use { input -> String(input.readUpTo(Lan.MOST_INFO)) }
                Lan.Info.of(text) ?: throw IOException("not a device's info")
            } finally {
                connection.disconnect()
            }
        } catch (failed: IOException) {
            null
        }
        if (answered == null) {
            if (running) toAll(me().copy(announce = false).toJson().toString().toByteArray())
            return
        }
        // (Its port and protocol as it announced them: an answer leaves them out. Its fingerprint as the connection
        // proved it where it was pinned to it.)
        heard(answered.copy(port = info.port, https = info.https), address, info.fingerprint.uppercase().takeIf { info.https })
    }
}

/** At most [most] bytes of what is left of this. */
fun java.io.InputStream.readUpTo(most: Int): ByteArray {
    val all = java.io.ByteArrayOutputStream()
    val some = ByteArray(8192)
    while (all.size() < most) {
        val read = read(some, 0, minOf(some.size, most - all.size()))
        if (read < 0) break
        all.write(some, 0, read)
    }
    return all.toByteArray()
}
