package dev.weiyi.glance.remote

import android.app.Activity
import android.app.Application
import android.content.Context
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.IOException
import java.io.InputStream
import java.net.BindException
import java.net.InetSocketAddress
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.cert.X509Certificate
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.SynchronousQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLServerSocket
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509TrustManager
import kotlin.concurrent.thread

/**
 * This phone as a whole LocalSend device (see `docs/nearby.md`, "Both ways"): while it receives (the app in front, or
 * 后台接收 on, or a sending being asked about or coming), it serves LocalSend v2 ([Serving]) and is found, and finds
 * devices, nearby ([Found]). The devices heard are kept for the sheet to send from. Started and stopped as what wants
 * it changes, one change at a time on a thread of its own.
 */
object Receiver {
    private lateinit var app: Context
    private val lock = Any()

    /** How many of the app's activities are started (one being made again as the screen turns counts on). */
    private var front = 0
    private var turning = 0
    /** Whether the receiving service is up ([ReceiveService]). */
    private var carrying = false

    private var serving: Serving? = null
    private var found: Found? = null
    private val changes = Executors.newSingleThreadExecutor { Thread(it, "nearby-settle").apply { isDaemon = true } }

    private val known = HashMap<String, Found.Device>()
    private val heardNow = MutableStateFlow<List<Found.Device>>(emptyList())
    /**
     * The devices heard lately (not this phone, not long ago), Glance's first, by name (as the PC lists them): each
     * keeps its place as it is heard again (it says itself every few seconds), so that the row about to be tapped is
     * not moved from under the finger.
     */
    val devices: StateFlow<List<Found.Device>> = heardNow

    private val receiving = MutableStateFlow(false)
    /** Whether this phone receives now. */
    val on: StateFlow<Boolean> = receiving

    /**
     * Begun with the app's process: its activities counted as they start and stop. Receiving in the background is not
     * started here (a process started for a job in the background may start no service in front): the service starts
     * as the phone does ([Booted]), with the app in front, and again by the system after its process ended.
     */
    fun begin(application: Application) {
        app = application
        application.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            override fun onActivityStarted(activity: Activity) {
                synchronized(lock) { if (turning > 0) turning-- else front++ }
                // (Receiving in the background, it is started again with the app in front, where the system lets it be.)
                if (Kept.background(activity) && ReceiveService.running == null) ReceiveService.start(activity)
                settle()
            }

            override fun onActivityStopped(activity: Activity) {
                synchronized(lock) { if (activity.isChangingConfigurations) turning++ else front-- }
                settle()
            }

            override fun onActivityCreated(activity: Activity, saved: Bundle?) {}
            override fun onActivityResumed(activity: Activity) {}
            override fun onActivityPaused(activity: Activity) {}
            override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) {}
            override fun onActivityDestroyed(activity: Activity) {}
        })
    }

    /** Whether the app is in front (an activity of its started). */
    fun inFront() = synchronized(lock) { front > 0 }

    /** The receiving service is up, or no longer. */
    fun carried(context: Context, on: Boolean) {
        synchronized(lock) { carrying = on }
        settle()
    }

    /**
     * What this phone says of itself to devices nearby: its name, its system and kind (as LocalSend's Android app says
     * them), its certificate, where it receives, and that it is a Glance.
     */
    fun me(context: Context) = Lan.Info(Own.alias(context), "Android", "mobile", Own.fingerprint(), serving?.port ?: Lan.PORT, https = true, glance = true)

    /** Started or stopped, as is wanted now. */
    fun settle() {
        changes.execute {
            val wanted = synchronized(lock) { front > 0 || carrying } || serving?.busy == true
            if (wanted && serving == null) start() else if (!wanted && serving != null) stop()
        }
    }

    private fun start() {
        val serving = try {
            Serving(app).also { it.start() }
        } catch (failed: IOException) {
            return
        }
        this.serving = serving
        found = try {
            Found(app, { me(app) }, ::heard).also { it.start() }
        } catch (failed: IOException) {
            null
        }
        receiving.value = true
        // The paired PCs asked where they are and by which certificate (what they send is taken by it), and told this
        // phone's (they know it nearby as their paired phone by it).
        thread(name = "nearby-pcs", isDaemon = true) {
            for (pc in Pcs.all(app)) {
                val at = Nearby.reach(app, pc) ?: continue
                runCatching { at.pc.said("/v1/nearby/trust", JSONObject().put("fingerprint", Own.fingerprint()).put("alias", Own.alias(app))) }
            }
        }
    }

    private fun stop() {
        serving?.close()
        serving = null
        found?.stop()
        found = null
        synchronized(known) { known.clear() }
        heardNow.value = emptyList()
        receiving.value = false
    }

    /** This phone announced to devices nearby (a sheet to send from opened: they answer, and are found). */
    fun announce() {
        changes.execute { found?.announce() }
    }

    /** A device heard of as it says it is, at [address] (by the certificate of [verified], where it proved one). */
    fun heard(info: Lan.Info, address: String, verified: String?) {
        val fingerprint = verified ?: info.fingerprint.uppercase()
        if (fingerprint.equals(Own.fingerprint(), ignoreCase = true)) return
        val device = Found.Device(fingerprint, info.alias, info.deviceModel, info.deviceType, info.glance, address, info.port ?: Lan.PORT, info.https, SystemClock.elapsedRealtime())
        synchronized(known) {
            // (Where it said nothing of its port and protocol, what it said before stands.)
            val was = known[fingerprint]
            known[fingerprint] = if (info.port == null && was != null) device.copy(port = was.port, https = was.https, glance = device.glance || was.glance) else device
            val now = SystemClock.elapsedRealtime()
            known.values.removeAll { now - it.heard > Found.FRESH }
            heardNow.value = known.values.sortedWith(compareBy<Found.Device>({ !it.glance }, { it.alias.lowercase() }, { it.fingerprint }))
        }
    }

    /** The sending coming now, if one is. */
    fun going(context: Context): Serving.Session? = serving?.going()

    /** Whether a device is being asked about. */
    fun asking(): Boolean = serving?.asking() == true

    /** A device's asking answered by this phone's user (from its notification). */
    fun answer(id: Int, take: Boolean, trust: Boolean) {
        serving?.answer(id, take, trust)
    }

    /** What is coming, cancelled by this phone's user: what came of it stays. */
    fun cancel() {
        serving?.cancel()
    }

    /**
     * Whether a sending taken may be received now: kept up in front of the system (the service up already, or
     * started now, the app being in front, where it may be). Else it is turned down.
     */
    fun carry(context: Context): Boolean {
        ReceiveService.running?.let { return true }
        if (!inFront()) return false
        ReceiveService.start(context)
        return true
    }

    /** What comes changed: the service's notification told, and whether receiving is still wanted. */
    fun changed() {
        ReceiveService.running?.let { service -> android.os.Handler(android.os.Looper.getMainLooper()).post { service.show() } }
        settle()
    }
}

/** The app's process: receiving from devices nearby follows its activities ([Receiver]). */
class GlanceApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Receiver.begin(this)
    }
}

/**
 * LocalSend v2's server, as this phone runs it while it receives: HTTPS by this phone's certificate (the one it sends
 * by, `glance-nearby`), on 53317 (another of the system's choosing where that is taken), asking for a client
 * certificate without needing one. Where one is presented, its fingerprint is the sender's, verified. One request per
 * connection, as the PC's helper does. One sending is received at a time, and one device asked about at a time.
 */
class Serving(private val context: Context) {
    companion object {
        /** How long a device's user is given to answer, and how long a receiving may wait for its next file (ms). */
        const val ASK_FOR = 60_000L
        private const val IDLE = 60_000L
        /** How long a connection may be quiet (ms). */
        private const val QUIET = 30_000
        /** How often the receiving's notification is told how far it is at most (ms). */
        private const val TELL_EVERY = 250L
        private val random = SecureRandom()

        private fun token(): String = ByteArray(16).also(random::nextBytes).joinToString("") { "%02x".format(it) }
    }

    /** Who sends: as it says it is, by the certificate it proved (or only said), and where it is. */
    class Sender(val alias: String, val model: String?, val type: String?, val glance: Boolean, val fingerprint: String, val verified: Boolean, val address: String)

    /** A device's sending asked about: its answer waited for. */
    class Asking(val id: Int, val sender: Sender, val offer: Lan.Offer) {
        val canTrust get() = sender.verified
        val answered = CountDownLatch(1)

        @Volatile
        var take = false

        @Volatile
        var trust = false
    }

    /** One file of a receiving: what was offered, where it goes, its token, and how it stands. */
    class Incoming(val offered: Lan.Offered, val place: Lan.Place, val token: String) {
        var writing = false
        var done = 0L
        var came: Kept.Came? = null
    }

    /** A receiving taken: its id, its sender, its files by their ids, and when anything of it last came. */
    class Session(val id: String, val sender: Sender, val files: Map<String, Incoming>) {
        var touched = SystemClock.elapsedRealtime()
        val total = files.values.sumOf { it.offered.size }
        val done get() = files.values.sumOf { if (it.came != null) it.offered.size else it.done }
        val whole get() = files.values.all { it.came != null }
    }

    private val lock = Any()
    private var session: Session? = null
    private var asking: Asking? = null
    private var told = 0L

    private val server: SSLServerSocket
    val port: Int

    @Volatile
    private var closed = false
    private val connections = ThreadPoolExecutor(0, 16, 30, TimeUnit.SECONDS, SynchronousQueue()) { Thread(it, "nearby-connection").apply { isDaemon = true } }
    private val idle = Executors.newSingleThreadScheduledExecutor { Thread(it, "nearby-idle").apply { isDaemon = true } }

    init {
        val tls = SSLContext.getInstance("TLS").apply { init(arrayOf(Own.Presenting()), arrayOf(AnyClient), null) }
        server = tls.serverSocketFactory.createServerSocket() as SSLServerSocket
        server.reuseAddress = true
        try {
            server.bind(InetSocketAddress(Lan.PORT))
        } catch (taken: BindException) {
            server.bind(InetSocketAddress(0))
        }
        server.wantClientAuth = true
        port = server.localPort
    }

    /** Takes the certificate any client presents (none is needed): it is known by its fingerprint, which TLS proved it holds. */
    private object AnyClient : X509TrustManager {
        override fun checkClientTrusted(chain: Array<out X509Certificate>, authType: String?) {}
        override fun checkServerTrusted(chain: Array<out X509Certificate>, authType: String?) = throw java.security.cert.CertificateException("this only receives")
        override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
    }

    fun start() {
        thread(name = "nearby-serve", isDaemon = true) {
            while (!closed) {
                val socket = try {
                    server.accept() as SSLSocket
                } catch (failed: IOException) {
                    if (closed) return@thread
                    continue
                }
                try {
                    connections.execute { connection(socket) }
                } catch (full: RejectedExecutionException) {
                    runCatching { socket.close() }
                }
            }
        }
        idle.scheduleWithFixedDelay(::idled, 10, 10, TimeUnit.SECONDS)
    }

    /** Serves no more: what comes is cut off, a device asked about is refused. */
    fun close() {
        closed = true
        runCatching { server.close() }
        idle.shutdownNow()
        synchronized(lock) { asking?.answered?.countDown() }
        val ended = synchronized(lock) { session.also { session = null } }
        ended?.let { end(it) }
    }

    /** Whether a sending is being asked about or received (receiving is kept up for it). */
    val busy get() = synchronized(lock) { session != null || asking != null }

    fun going(): Session? = synchronized(lock) { session }

    fun asking(): Boolean = synchronized(lock) { asking != null }

    private fun connection(socket: SSLSocket) {
        try {
            socket.soTimeout = QUIET
            socket.startHandshake()
            val verified = try {
                (socket.session.peerCertificates.firstOrNull() as? X509Certificate)?.let(Own::fingerprint)
            } catch (none: javax.net.ssl.SSLPeerUnverifiedException) {
                null
            }
            val address = socket.inetAddress.hostAddress.orEmpty()
            val input = BufferedInputStream(socket.inputStream)
            val output = BufferedOutputStream(socket.outputStream)
            val (status, json) = try {
                val request = Lan.request(input) ?: return
                asked(request, address, verified)
            } catch (bad: Lan.Bad) {
                bad.status to JSONObject().put("message", bad.message).toString()
            }
            Lan.answer(output, status, json)
        } catch (failed: IOException) {
            // (Broken off, or never a TLS connection: nothing to answer.)
        } finally {
            runCatching { socket.close() }
        }
    }

    /** What [request] (from [address], which proved the certificate of [verified], if any) asks, done: the answer's status and body. */
    private fun asked(request: Lan.Request, address: String, verified: String?): Pair<Int, String?> {
        val path = request.path.takeIf { it.startsWith(Lan.API) }?.removePrefix(Lan.API) ?: throw Lan.Bad(404, "no such thing")
        return when (request.method to path) {
            "GET" to "/info" -> {
                if (request.query["fingerprint"].equals(Own.fingerprint(), ignoreCase = true)) throw Lan.Bad(412, "Self-discovered")
                200 to about()
            }
            "POST" to "/register" -> {
                val info = Lan.Info.of(request.text(Lan.MOST_INFO)) ?: throw Lan.Bad(400, "not a device's info")
                Receiver.heard(info, address, verified)
                200 to about()
            }
            "POST" to "/prepare-upload" -> prepare(Lan.Offer.of(request.text(Lan.MOST_OFFER)), address, verified)
            "POST" to "/upload" -> upload(request, address).let { 200 to null }
            "POST" to "/cancel" -> cancelled(request.query["sessionId"], address).let { 200 to null }
            else -> throw Lan.Bad(404, "no such thing")
        }
    }

    /** This phone as it answers `register` and `info`. */
    private fun about(): String = Receiver.me(context).toJson().toString()

    /**
     * A sending offered (`prepare-upload`): taken (from a paired PC or a device trusted here, or its user says so), a
     * receiving begun and each file's token given. Else why not. A message (LocalSend's words: one text "file" whose
     * words are its preview) is taken as words: nothing of it is uploaded (answered 204), and it is shown at once.
     */
    private fun prepare(offer: Lan.Offer, address: String, verified: String?): Pair<Int, String?> {
        val sender = Sender(offer.info.alias, offer.info.deviceModel, offer.info.deviceType, offer.info.glance, verified ?: offer.info.fingerprint.uppercase(), verified != null, address)
        if (offer.files.isEmpty()) return 204 to null
        val words = offer.words
        if (words == null) {
            if (!Store.writable(context)) throw Lan.Bad(403, "files cannot be kept")
            if (offer.total > Store.room()) throw Lan.Bad(403, "no room for them")
            synchronized(lock) { if (session != null) throw Lan.Bad(409, "another sending is being received") }
        }
        val paired = Pcs.fingerprints(context).values.toSet()
        val trusted = Kept.trusted(context).map { it.fingerprint }.toSet()
        when (val decision = Lan.decide(verified, paired, trusted, Kept.others(context))) {
            Lan.Decision.Take -> {}
            Lan.Decision.Refuse -> throw Lan.Bad(403, Lan.NOT_TRUSTED)
            is Lan.Decision.Ask -> {
                val asked = synchronized(lock) {
                    if (asking != null) throw Lan.Bad(409, "another device is being asked about")
                    Asking(Kept.number(), sender, offer).also { asking = it }
                }
                try {
                    if (Notices.ask(context, asked)) asked.answered.await(ASK_FOR, TimeUnit.MILLISECONDS)
                } finally {
                    synchronized(lock) { asking = null }
                    Notices.asked(context)
                    Receiver.changed()
                }
                if (!asked.take || closed) throw Lan.Bad(403, Lan.REFUSED)
                if (asked.trust && decision.canTrust) Kept.trust(context, sender)
            }
        }
        if (words != null) {
            val came = Kept.Came(Kept.number().toLong(), words, words.lineSequence().first().take(80), "text/plain", null, words.length.toLong(), null, sender.alias, System.currentTimeMillis())
            Kept.came(context, listOf(came))
            Notices.words(context, sender.alias, came)
            return 204 to null
        }
        val files = offer.files.associate { it.id to Incoming(it, Store.place(it.name), token()) }
        val begun = Session(token(), sender, files)
        synchronized(lock) {
            if (session != null) throw Lan.Bad(409, "another sending is being received")
            if (closed) throw Lan.Bad(403, Lan.REFUSED)
            session = begun
        }
        // (Kept up in front of the system while it comes, or not taken.)
        if (!Receiver.carry(context)) {
            synchronized(lock) { if (session === begun) session = null }
            throw Lan.Bad(403, "not receiving now")
        }
        Receiver.changed()
        val tokens = JSONObject()
        for ((id, file) in files) tokens.put(id, file.token)
        return 200 to JSONObject().put("sessionId", begun.id).put("files", tokens).toString()
    }

    /**
     * One file of the receiving going on (`upload`), as its body comes: written where it is kept, checked to be as long
     * as it was said to be (and, where its hash was said, to be that). Given up where the receiving is cancelled
     * meanwhile.
     */
    private fun upload(request: Lan.Request, address: String) {
        val (id, fileId, token) = Triple(request.query["sessionId"].orEmpty(), request.query["fileId"].orEmpty(), request.query["token"].orEmpty())
        val (going, file) = synchronized(lock) {
            val going = session?.takeIf { it.id == id } ?: throw Lan.Bad(403, "no such receiving")
            if (going.sender.address != address) throw Lan.Bad(403, "not from where the sending was offered")
            val file = going.files[fileId]?.takeIf { it.token == token } ?: throw Lan.Bad(403, "no such file")
            if (file.writing || file.came != null) throw Lan.Bad(409, "that file is coming already")
            file.writing = true
            going.touched = SystemClock.elapsedRealtime()
            going to file
        }
        val came = try {
            written(request.body, going, file)
        } finally {
            synchronized(lock) {
                file.writing = false
                if (file.came == null) file.done = 0
            }
        }
        val ended = synchronized(lock) {
            if (session !== going) {
                // (Cancelled as it ended: what came goes with it.)
                null
            } else {
                file.came = came
                going.touched = SystemClock.elapsedRealtime()
                if (going.whole) going.also { session = null } else null
            }
        }
        if (ended != null) end(ended) else Receiver.changed()
    }

    /** [file]'s body, written where it is kept and, whole and as its hash says, kept: what came. */
    private fun written(body: InputStream, going: Session, file: Incoming): Kept.Came {
        val saving = try {
            Store.open(context, file.place)
        } catch (failed: Exception) {
            throw Lan.Bad(500, "the file cannot be written")
        }
        val hash = file.offered.sha256?.let { MessageDigest.getInstance("SHA-256") }
        val size = file.offered.size
        try {
            val some = ByteArray(256 * 1024)
            var done = 0L
            while (true) {
                val read = try {
                    body.read(some)
                } catch (broken: IOException) {
                    throw Lan.Bad(400, "the file did not come whole")
                }
                if (read < 0) break
                if (done + read > size) throw Lan.Bad(400, "more than was said")
                synchronized(lock) { if (session !== going) throw Lan.Bad(403, "the receiving was cancelled") }
                try {
                    saving.output.write(some, 0, read)
                } catch (failed: IOException) {
                    throw Lan.Bad(500, "the file cannot be written")
                }
                hash?.update(some, 0, read)
                done += read
                synchronized(lock) {
                    file.done = done
                    going.touched = SystemClock.elapsedRealtime()
                }
                val now = SystemClock.elapsedRealtime()
                if (now - told >= TELL_EVERY) {
                    told = now
                    Receiver.changed()
                }
            }
            if (done != size) throw Lan.Bad(400, "the file did not come whole")
            if (hash != null && hash.digest().joinToString("") { "%02x".format(it) } != file.offered.sha256) throw Lan.Bad(422, "not as its hash says")
            Store.finish(context, saving)
        } catch (failed: Lan.Bad) {
            Store.abandon(context, saving)
            throw failed
        } catch (failed: Exception) {
            Store.abandon(context, saving)
            throw Lan.Bad(500, "the file cannot be written")
        }
        return Kept.Came(Kept.number().toLong(), null, file.place.name, file.place.type, saving.uri.toString(), size, file.place.shelf.name, going.sender.alias, System.currentTimeMillis())
    }

    /**
     * A sending cancelled by its device: the one going on (named, or the device's, unnamed: LocalSend's earlier way),
     * or, before any is, the asking about it.
     */
    private fun cancelled(id: String?, address: String) {
        val ended = synchronized(lock) {
            val going = session
            if (going != null && (id == null || going.id == id) && going.sender.address == address) {
                session = null
                going
            } else {
                if (id == null) asking?.takeIf { it.sender.address == address }?.answered?.countDown()
                null
            }
        }
        ended?.let { end(it) }
    }

    /** A device's asking answered (by its id): taken or not, and its device trusted or not. */
    fun answer(id: Int, take: Boolean, trust: Boolean) {
        synchronized(lock) {
            val asked = asking?.takeIf { it.id == id } ?: return
            asked.take = take
            asked.trust = trust
            asked.answered.countDown()
        }
    }

    /** What is coming, cancelled here: what came of it stays. */
    fun cancel() {
        val ended = synchronized(lock) { session.also { session = null } } ?: return
        end(ended)
    }

    /** A receiving that waited too long for its next file, given up. */
    private fun idled() {
        val ended = synchronized(lock) {
            session?.takeIf { going -> SystemClock.elapsedRealtime() - going.touched > IDLE && going.files.values.none { it.writing } }?.also { session = null }
        }
        ended?.let { end(it) }
    }

    /** [ended] over (whole, or cut short): what came of it told and listed, and receiving kept up only as it is still wanted. */
    private fun end(ended: Session) {
        val came = synchronized(lock) { ended.files.values.mapNotNull { it.came } }
        if (came.isNotEmpty()) {
            Kept.came(context, came)
            Notices.came(context, ended.sender.alias, came)
        }
        Receiver.changed()
    }
}
