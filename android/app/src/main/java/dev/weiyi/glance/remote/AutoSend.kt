package dev.weiyi.glance.remote

import android.app.ActivityManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.MediaStore
import android.provider.Settings
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException
import java.math.BigInteger
import java.net.Socket
import java.net.URL
import java.net.URLEncoder
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.Principal
import java.security.PrivateKey
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.security.spec.ECGenParameterSpec
import java.util.Date
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLEngine
import javax.net.ssl.X509ExtendedKeyManager
import javax.net.ssl.X509TrustManager
import javax.security.auth.x500.X500Principal
import kotlin.concurrent.thread

/**
 * This phone as it sends to a PC nearby: a certificate of its own, made once in the system's key store (its key never
 * leaves it), presented whenever it sends. The PC knows the phone by its fingerprint, told it over the signed pairing
 * channel (see `/v1/nearby/trust`), and so takes what it sends without asking.
 */
object Own {
    private const val ALIAS = "glance-nearby"

    private fun store() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    /** The certificate, made the first time it is wanted. */
    @Synchronized
    fun certificate(): X509Certificate {
        (store().getCertificate(ALIAS) as X509Certificate?)?.let { return it }
        val now = System.currentTimeMillis()
        val day = 24 * 60 * 60 * 1000L
        // (Signed by its own key: the key store does so for a key that may sign. TLS signs with it as 1.2 and 1.3 do.)
        val spec = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_SIGN)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_NONE, KeyProperties.DIGEST_SHA256, KeyProperties.DIGEST_SHA384, KeyProperties.DIGEST_SHA512)
            .setCertificateSubject(X500Principal("CN=Glance"))
            .setCertificateSerialNumber(BigInteger.valueOf(now))
            .setCertificateNotBefore(Date(now - day))
            .setCertificateNotAfter(Date(now + 20 * 365 * day))
            .build()
        KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore").apply { initialize(spec) }.generateKeyPair()
        return store().getCertificate(ALIAS) as X509Certificate
    }

    fun key(): PrivateKey = store().getKey(ALIAS, null) as PrivateKey

    /** Its fingerprint, as LocalSend's devices know one another: the SHA-256 of the certificate, in capital hex. */
    fun fingerprint(): String = fingerprint(certificate())

    fun fingerprint(certificate: X509Certificate): String = MessageDigest.getInstance("SHA-256").digest(certificate.encoded).joinToString("") { "%02X".format(it) }

    /** What this phone is called, as its user named it (else as it is made). */
    fun alias(context: Context): String = Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)?.takeIf { it.isNotBlank() } ?: Phone.name

    /** Presents the certificate to whoever asks for one. */
    class Presenting : X509ExtendedKeyManager() {
        private val certificate = certificate()
        private val key = key()

        override fun chooseClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, socket: Socket?) = ALIAS
        override fun chooseEngineClientAlias(keyType: Array<out String>?, issuers: Array<out Principal>?, engine: SSLEngine?) = ALIAS
        override fun getClientAliases(keyType: String?, issuers: Array<out Principal>?) = arrayOf(ALIAS)
        override fun getCertificateChain(alias: String?) = arrayOf(certificate)
        override fun getPrivateKey(alias: String?) = key
        override fun chooseServerAlias(keyType: String?, issuers: Array<out Principal>?, socket: Socket?): String? = null
        override fun getServerAliases(keyType: String?, issuers: Array<out Principal>?): Array<String>? = null
    }
}

/**
 * Talks on only to the PC that presents the certificate of [fingerprint] (its name and issuer say nothing: every
 * device signs its own). The fingerprint is the one the PC told over the signed pairing channel.
 */
private class Pinned(private val fingerprint: String) : X509TrustManager {
    override fun checkServerTrusted(chain: Array<out X509Certificate>, authType: String?) {
        if (chain.isEmpty() || !Own.fingerprint(chain[0]).equals(fingerprint, ignoreCase = true)) throw CertificateException("not the certificate the PC told")
    }

    override fun checkClientTrusted(chain: Array<out X509Certificate>, authType: String?) = throw CertificateException("this does not receive")

    override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
}

/**
 * New photos (and screenshots, and videos, as chosen) sent to one paired PC by themselves, the app not opened, as
 * LocalSend sends (see `docs/nearby.md`): what is taken is noticed by the system's content triggers ([PhotoWatch]),
 * kept in a queue of this app's, and sent when there is a network to send on ([PhotoSend]), as much of what waits as
 * the PC has room for in one sending. What the PC took leaves the queue, each file as it is taken; what its user
 * refused, too; the rest waits and is tried again.
 */
object AutoSend {
    private const val WATCH = 7301
    private const val SEND = 7302
    private const val NOTICE = 7303
    private const val CHANNEL = "auto_send"

    /**
     * How far before the last look a look goes back (s): what the system had pending then (being written: a video
     * being taken) is published later with the time it was added, and is still found.
     */
    private const val BEFORE = 24 * 60 * 60L

    /**
     * How long after it was added a photo the camera took is found at most where the app is let be in the background
     * (s): its watch is told within 8 s. Found later, the system held the app there.
     */
    private const val LATE = 60L

    /** What a PC's user said in refusing a sending (see the helper's `prepare`): it is not offered again. */
    private val REFUSALS = setOf("refused", "not taken from devices not trusted")

    /** Held while what is kept is changed, and while the sending job is taken up or let go. */
    private val lock = Any()

    /** One run of the sending job: told when it is to end (the system wants it to, or what it sends for changed). */
    class Run {
        @Volatile
        var stopped = false
    }

    /** The run sending now, if one is: it takes what is queued meanwhile before it ends. */
    private var sender: Run? = null

    /** [run] is the one sending from now on: taken up as its job starts, before anything of an earlier one may. */
    fun begin(run: Run) = synchronized(lock) { sender = run }

    private fun kept(context: Context) = context.getSharedPreferences("auto_send", Context.MODE_PRIVATE)

    /** The key of the PC sent to, if any is. */
    fun to(context: Context): String? = kept(context).getString("to", null)

    fun screenshots(context: Context) = kept(context).getBoolean("screenshots", true)

    fun videos(context: Context) = kept(context).getBoolean("videos", false)

    fun notice(context: Context) = kept(context).getBoolean("notice", true)

    /**
     * Whether the photos taken lately were found only long after ([LATE]), so that the system holds the app in the
     * background where it said it lets it be there (some, as ColorOS, keep a setting of their own): until one is found in
     * time with the app not in front.
     */
    fun held(context: Context) = kept(context).getBoolean("held", false)

    /** Whether the app is in front now (its jobs run then wherever it is held otherwise). */
    private fun inFront(): Boolean {
        val info = ActivityManager.RunningAppProcessInfo()
        ActivityManager.getMyMemoryState(info)
        return info.importance <= ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
    }

    private fun now() = System.currentTimeMillis() / 1000

    /** The permission the phone's [kind] of media (`i` pictures, `v` videos) are read by. */
    fun permission(kind: Char) = when {
        Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU -> android.Manifest.permission.READ_EXTERNAL_STORAGE
        kind == 'v' -> android.Manifest.permission.READ_MEDIA_VIDEO
        else -> android.Manifest.permission.READ_MEDIA_IMAGES
    }

    /** Whether all of the phone's [kind] of media may be read (not only some chosen: then new ones are not seen). */
    fun readable(context: Context, kind: Char) = context.checkSelfPermission(permission(kind)) == PackageManager.PERMISSION_GRANTED

    /**
     * Whether the system lets this app be in the background as it needs to: where it is optimised for the battery,
     * the system may hold it there (some, as ColorOS, freeze it), and its jobs do not run till it is opened.
     */
    fun unheld(context: Context) = context.getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(context.packageName)

    /** What asks its user to let this app be in the background ([unheld]): the system's own question. */
    fun letGo(context: Context) = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:${context.packageName}"))

    /** The system's settings for this app: where a system holding it otherwise ([held]) lets it be in the background. */
    fun settings(context: Context) = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))

    /**
     * Sends to [pc] from now on (what was taken before is not sent): the PC asked where it receives, and told this
     * phone's certificate. Blocks: not from the main thread. Throws what asking the PC throws, and [Off] where the PC
     * does not receive.
     */
    fun turnOn(context: Context, pc: Pc) {
        if (!pc.said("/v1/nearby").optBoolean("on")) throw Off
        pc.said("/v1/nearby/trust", JSONObject().put("fingerprint", Own.fingerprint()).put("alias", Own.alias(context)))
        synchronized(lock) {
            kept(context).edit().putString("to", pc.key).putLong("since", now()).putLong("looked_i", now()).putLong("looked_v", now()).putLong("videos_since", now()).remove("seen").remove("queue").remove("held").commit()
            // (What was being sent to another PC is not.)
            sender?.stopped = true
            context.getSystemService(JobScheduler::class.java).cancel(SEND)
        }
        arm(context)
    }

    /** The PC does not receive from devices nearby (its settings have it off). */
    object Off : Exception()

    /** Sends no more: what waits is not sent. */
    fun turnOff(context: Context) {
        synchronized(lock) {
            kept(context).edit().remove("to").remove("seen").remove("queue").commit()
            sender?.stopped = true
            val jobs = context.getSystemService(JobScheduler::class.java)
            jobs.cancel(WATCH)
            jobs.cancel(SEND)
        }
    }

    /** The PC with [key] is unpaired: sent to no more if it was. */
    fun unpaired(context: Context, key: String) {
        if (to(context) == key) turnOff(context)
    }

    fun setScreenshots(context: Context, on: Boolean) = kept(context).edit().putBoolean("screenshots", on).apply()

    fun setNotice(context: Context, on: Boolean) = kept(context).edit().putBoolean("notice", on).apply()

    /** Videos sent too (those taken from now on), or not. */
    fun setVideos(context: Context, on: Boolean) {
        synchronized(lock) { kept(context).edit().putBoolean("videos", on).putLong("videos_since", now()).commit() }
        if (to(context) != null) arm(context)
    }

    /**
     * Where it is on, what is taken is noticed again and what was taken meanwhile looked at (the phone started, the
     * app was updated or opened: the system forgets the jobs of an app stopped by its user). Blocks: not from the
     * main thread.
     */
    fun resume(context: Context) {
        if (to(context) == null) return
        arm(context)
        look(context)
    }

    /** What is taken is noticed (again: as the phone starts, and after each time it was). */
    fun arm(context: Context) {
        val watch = JobInfo.Builder(WATCH, ComponentName(context, PhotoWatch::class.java))
            .addTriggerContentUri(JobInfo.TriggerContentUri(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, JobInfo.TriggerContentUri.FLAG_NOTIFY_FOR_DESCENDANTS))
            // (A burst of photos is told as one: sent together.)
            .setTriggerContentUpdateDelay(1500)
            .setTriggerContentMaxDelay(8000)
        if (videos(context)) watch.addTriggerContentUri(JobInfo.TriggerContentUri(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, JobInfo.TriggerContentUri.FLAG_NOTIFY_FOR_DESCENDANTS))
        context.getSystemService(JobScheduler::class.java).schedule(watch.build())
    }

    /** One of the phone's pictures or videos, by its place in the system's media. */
    private class Entry(val kind: Char, val id: Long) {
        val name get() = "$kind$id"
        val uri: Uri get() = ContentUris.withAppendedId(if (kind == 'v') MediaStore.Video.Media.EXTERNAL_CONTENT_URI else MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id)

        companion object {
            fun of(name: String) = name.drop(1).toLongOrNull()?.let { Entry(name[0], it) }
        }
    }

    private fun queue(context: Context): List<Entry> {
        val list = JSONArray(kept(context).getString("queue", "[]"))
        return (0 until list.length()).mapNotNull { Entry.of(list.getString(it)) }
    }

    /** What was added lately and has been looked at (sent or not), with when it was added. */
    private fun seen(context: Context): JSONObject = JSONObject(kept(context).getString("seen", "{}")!!)

    /**
     * What was added since it was last looked at (each kind by itself: from a while before its last look, see
     * [BEFORE]), queued where it is to be sent:
     * taken by the camera (in DCIM), or a screen's capture where those are sent. Then sent, unless a run is sending
     * already (it takes it). Without access to the photos nothing is looked at, so nothing is passed over: it is,
     * once access is given again.
     */
    fun look(context: Context): Unit = synchronized(lock) {
        if (to(context) == null || !readable(context, 'i')) return
        val seen = seen(context)
        val queue = queue(context).map { it.name }.toMutableList()
        val start = now()
        val since = kept(context).getLong("since", start)
        // (Videos only while they are sent and may be read: where they may not, their last look stays as it was.)
        val kinds = listOf('i' to since) + if (videos(context) && readable(context, 'v')) listOf('v' to maxOf(since, kept(context).getLong("videos_since", start))) else listOf()
        val screenshots = screenshots(context)
        val edit = kept(context).edit()
        // Whether a photo the camera took was found long after it was added, and whether one was found in time.
        var late = false
        var prompt = false
        for ((kind, from) in kinds) {
            for (added in added(context, kind, maxOf(from, kept(context).getLong("looked_$kind", start) - BEFORE))) {
                if (seen.has(added.entry.name)) continue
                seen.put(added.entry.name, added.at)
                if (added.camera || (added.screen && screenshots)) queue += added.entry.name
                // (Pictures alone: a video is added as it begins to be taken.)
                if (added.camera && kind == 'i') if (start - added.at > LATE) late = true else prompt = true
            }
            // (What of it was added before its next look goes back to is forgotten: it is not looked at again.)
            seen.keys().asSequence().filter { it[0] == kind && seen.getLong(it) < start - BEFORE }.toList().forEach(seen::remove)
            edit.putLong("looked_$kind", start)
        }
        // Found late, the app was held. Found in time with the app not in front, it is not (in front, its jobs run anyway).
        if (late) edit.putBoolean("held", true) else if (prompt && !inFront()) edit.putBoolean("held", false)
        edit.putString("seen", seen.toString()).putString("queue", JSONArray(queue).toString()).commit()
        if (queue.isNotEmpty() && sender.let { it == null || it.stopped }) send(context)
    }

    private fun send(context: Context) {
        val job = JobInfo.Builder(SEND, ComponentName(context, PhotoSend::class.java))
            // (Where the PC is: a phone's own network, not its mobile data.)
            .setRequiredNetworkType(JobInfo.NETWORK_TYPE_UNMETERED)
            .setBackoffCriteria(30_000, JobInfo.BACKOFF_POLICY_EXPONENTIAL)
            .setPersisted(true)
        context.getSystemService(JobScheduler::class.java).schedule(job.build())
    }

    /** Something added: where it is kept, when it was added (s), and whether the camera took it or it is of the screen. */
    private class Added(val entry: Entry, val at: Long, val camera: Boolean, val screen: Boolean)

    /** What of [kind] was added since [from] (s), as the system's media have it (what is still being written is not there yet). */
    private fun added(context: Context, kind: Char, from: Long): List<Added> {
        val collection = if (kind == 'v') MediaStore.Video.Media.EXTERNAL_CONTENT_URI else MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        // Where it is: its folder under the storage's root (Android 10 on), else its path and its folder's name.
        val relative = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q
        @Suppress("DEPRECATION")
        val projection = arrayOf(MediaStore.MediaColumns._ID, MediaStore.MediaColumns.DATE_ADDED) +
            if (relative) arrayOf(MediaStore.MediaColumns.RELATIVE_PATH) else arrayOf(MediaStore.MediaColumns.DATA, MediaStore.Images.ImageColumns.BUCKET_DISPLAY_NAME)
        val found = mutableListOf<Added>()
        context.contentResolver.query(collection, projection, "${MediaStore.MediaColumns.DATE_ADDED} >= ?", arrayOf(from.toString()), "${MediaStore.MediaColumns._ID} ASC")?.use { rows ->
            while (rows.moveToNext()) {
                val (camera, screen) = if (relative) {
                    val path = rows.getString(2).orEmpty()
                    path.startsWith("DCIM/", ignoreCase = true) to path.contains("screen", ignoreCase = true)
                } else {
                    // (Before Android 10, a screen's capture is told by its folder's name.)
                    rows.getString(2).orEmpty().contains("/DCIM/", ignoreCase = true) to rows.getString(3).orEmpty().contains("screen", ignoreCase = true)
                }
                found += Added(Entry(kind, rows.getLong(0)), rows.getLong(1), camera && !screen, screen)
            }
        }
        return found
    }

    /** One file to send: what it is in the queue, and what the PC is told of it. */
    private class Going(val entry: Entry, val name: String, val type: String, val size: Long)

    /** What [entry] is now (read with access to its kind): none where it is gone (deleted, or in the trash, since). */
    private fun going(context: Context, entry: Entry): Going? {
        // (The trash is asked of from Android 11 on: an item asked for by its place is found though it is there.)
        val trash = Build.VERSION.SDK_INT >= Build.VERSION_CODES.R
        val projection = arrayOf(MediaStore.MediaColumns.DISPLAY_NAME, MediaStore.MediaColumns.MIME_TYPE) + if (trash) arrayOf(MediaStore.MediaColumns.IS_TRASHED) else arrayOf()
        val (name, type) = context.contentResolver.query(entry.uri, projection, null, null, null)?.use { rows ->
            if (rows.moveToFirst() && !(trash && rows.getInt(2) == 1)) rows.getString(0).orEmpty() to (rows.getString(1) ?: "application/octet-stream") else null
        } ?: return null
        val size = try {
            context.contentResolver.openFileDescriptor(entry.uri, "r")?.use { it.statSize } ?: return null
        } catch (gone: java.io.FileNotFoundException) {
            return null
        }
        return Going(entry, name.ifBlank { entry.name }, type, size)
    }

    /** How a sending went: all of it gone (or given up), or to be tried again later. */
    private enum class Sent { Done, Again }

    /**
     * Sends what is queued, till nothing is (what is queued meanwhile, too), as [run]: whether to be tried again later
     * (the PC not reached, busy, or without room yet). It ends at once when [run] is stopped, its sending cancelled.
     * What may not be read now (access taken away) stays queued, unsent, till it may. [run] was begun ([begin]).
     */
    fun sendAll(context: Context, run: Run): Boolean {
        try {
            while (!run.stopped) {
                val (pc, batch) = synchronized(lock) {
                    val pc = Pcs.all(context).firstOrNull { it.key == to(context) }
                    val batch = queue(context).filter { readable(context, it.kind) }
                    // (Let go at once with nothing to send: what is queued after is sent by another run.)
                    if (pc == null || batch.isEmpty()) {
                        if (sender === run) sender = null
                        return false
                    }
                    pc to batch
                }
                if (sendOnce(context, pc, batch, run) == Sent.Again) return true
            }
            return true
        } finally {
            synchronized(lock) { if (sender === run) sender = null }
        }
    }

    /** What has gone, or was given up, leaves the queue. */
    private fun done(context: Context, entries: List<Entry>) = synchronized(lock) {
        val names = entries.map { it.name }.toSet()
        kept(context).edit().putString("queue", JSONArray(queue(context).map { it.name }.filter { it !in names }).toString()).commit()
    }

    /** Where [pc] receives, as it says over the signed channel: looked for on the network where it is not at its address. */
    private fun nearby(context: Context, pc: Pc): Pair<Pc, JSONObject>? {
        try {
            return pc to pc.said("/v1/nearby")
        } catch (failed: Failure.Failed) {
            return null
        } catch (failure: Failure) {
            // Not answered, or answered by one that does not know this phone's key (another machine at its old
            // address): the PC itself is looked for.
        }
        val host = pc.find()?.takeIf { it != pc.host } ?: return null
        Pcs.move(context, pc.key, host)
        val moved = pc.copy(host = host)
        return try { moved to moved.said("/v1/nearby") } catch (failure: Failure) { null }
    }

    private fun sendOnce(context: Context, paired: Pc, batch: List<Entry>, run: Run): Sent {
        val files = batch.associateWith { going(context, it) }
        done(context, files.filterValues { it == null }.keys.toList())
        val going = files.values.filterNotNull()
        if (going.isEmpty()) return Sent.Done
        val (pc, nearby) = nearby(context, paired) ?: return Sent.Again
        val port = nearby.optInt("port")
        val fingerprint = nearby.optString("fingerprint")
        if (!nearby.optBoolean("on") || port !in 1..65535 || fingerprint.length != 64) return Sent.Again
        // As much as the PC has room for, in their order (what is larger waits till it has), and as many as one sending holds.
        val mostFile = nearby.optLong("most_file")
        val mostSending = if (nearby.isNull("most_sending")) Long.MAX_VALUE else nearby.optLong("most_sending")
        val mostFiles = nearby.optInt("most_files")
        var total = 0L
        val fit = going.filter { file -> (file.size <= mostFile && total + file.size <= mostSending).also { if (it) total += file.size } }.take(mostFiles)
        if (fit.isEmpty() || run.stopped) return Sent.Again
        val tls = SSLContext.getInstance("TLS").apply { init(arrayOf(Own.Presenting()), arrayOf(Pinned(fingerprint)), null) }
        fun open(path: String): HttpsURLConnection = (URL("https://${pc.host}:$port/api/localsend/v2$path").openConnection() as HttpsURLConnection).apply {
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
            .put("files", JSONObject().apply { for (file in fit) put(file.entry.name, JSONObject().put("id", file.entry.name).put("fileName", file.name).put("size", file.size).put("fileType", file.type)) })
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
            return Sent.Again
        }
        val said = runCatching { JSONObject(answer) }.getOrNull()
        when {
            status == 200 && said != null -> {}
            // Nothing wanted of them.
            status == 204 -> return Sent.Done.also { done(context, fit.map { it.entry }) }
            // Refused by the PC's user (or a PIN wanted, which Glance never wants): given up.
            status == 401 || (status == 403 && said?.optString("message") in REFUSALS) -> return Sent.Done.also { done(context, fit.map { it.entry }) }
            // Busy receiving another, without room since it said, or anything else: later.
            else -> return Sent.Again
        }
        val session = said!!.optString("sessionId")
        val tokens = said.optJSONObject("files") ?: JSONObject()
        fun cancel() = runCatching { open("/cancel?sessionId=${URLEncoder.encode(session, "UTF-8")}").apply { requestMethod = "POST" }.let { it.responseCode; it.disconnect() } }
        var sent = 0
        var whole = true
        for (file in fit) {
            val token = tokens.optString(file.entry.name)
            // (One the PC does not want is not sent: it has it.)
            if (token.isEmpty()) {
                done(context, listOf(file.entry))
                continue
            }
            val query = listOf("sessionId" to session, "fileId" to file.entry.name, "token" to token).joinToString("&") { (name, value) -> "$name=${URLEncoder.encode(value, "UTF-8")}" }
            val ok = !run.stopped && try {
                val upload = open("/upload?$query")
                try {
                    upload.requestMethod = "POST"
                    upload.doOutput = true
                    upload.setFixedLengthStreamingMode(file.size)
                    upload.setRequestProperty("Content-Type", "application/octet-stream")
                    val from = context.contentResolver.openInputStream(file.entry.uri) ?: throw IOException("gone")
                    from.use { input ->
                        upload.outputStream.use { output ->
                            val some = ByteArray(64 * 1024)
                            while (true) {
                                if (run.stopped) throw IOException("stopped")
                                val read = input.read(some)
                                if (read < 0) break
                                output.write(some, 0, read)
                            }
                        }
                    }
                    upload.responseCode == 200
                } finally {
                    upload.disconnect()
                }
            } catch (failed: IOException) {
                false
            }
            if (!ok) {
                // The PC keeps what it took of this sending; the rest is offered again later.
                cancel()
                whole = false
                break
            }
            // Taken: it leaves the queue at once, so that it is not sent again whatever comes next.
            done(context, listOf(file.entry))
            sent++
        }
        if (sent > 0 && notice(context)) tell(context, pc.name, sent)
        return if (whole) Sent.Done else Sent.Again
    }

    /** That [count] went to [pc]: a notice that goes by itself after a while. */
    private fun tell(context: Context, pc: String, count: Int) {
        val notices = context.getSystemService(NotificationManager::class.java)
        if (!notices.areNotificationsEnabled()) return
        notices.createNotificationChannel(NotificationChannel(CHANNEL, context.getString(R.string.auto_channel), NotificationManager.IMPORTANCE_LOW))
        val text = if (count == 1) context.getString(R.string.auto_sent, pc) else context.resources.getQuantityString(R.plurals.auto_sent_many, count, pc, count)
        val notice = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_sent)
            .setContentTitle(text)
            .setAutoCancel(true)
            .setTimeoutAfter(6000)
            .setContentIntent(android.app.PendingIntent.getActivity(context, 0, Intent(context, MainActivity::class.java), android.app.PendingIntent.FLAG_IMMUTABLE))
            .build()
        notices.notify(NOTICE, notice)
    }
}

/** What the camera (or the screen) added: noticed, queued and sent; then noticed again. */
class PhotoWatch : JobService() {
    override fun onStartJob(params: JobParameters): Boolean {
        thread(name = "auto-send-look") {
            AutoSend.look(this)
            if (AutoSend.to(this) != null) AutoSend.arm(this)
            jobFinished(params, false)
        }
        return true
    }

    override fun onStopJob(params: JobParameters) = false
}

/** What is queued, sent when there is a network to send on. */
class PhotoSend : JobService() {
    /** The run going on (the system runs one of this job at a time: one stopped is told so before the next begins). */
    @Volatile
    private var run: AutoSend.Run? = null

    override fun onStartJob(params: JobParameters): Boolean {
        val run = AutoSend.Run().also { run = it }
        AutoSend.begin(run)
        thread(name = "auto-send") {
            val again = AutoSend.sendAll(this, run)
            if (!run.stopped) jobFinished(params, again)
        }
        return true
    }

    override fun onStopJob(params: JobParameters): Boolean {
        run?.stopped = true
        return true
    }
}

/** As the phone starts (and as the app is updated), what is taken is noticed again: the system forgets it. */
class Booted : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED && intent.action != Intent.ACTION_MY_PACKAGE_REPLACED) return
        val pending = goAsync()
        thread(name = "auto-send-boot") {
            AutoSend.resume(context)
            pending.finish()
        }
    }
}
