package dev.weiyi.glance.remote

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.content.res.AssetFileDescriptor
import android.net.Uri
import android.os.Build
import android.os.IBinder
import android.provider.OpenableColumns
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.util.UUID
import kotlin.concurrent.thread

/**
 * What its user hands over to a paired PC by hand: photos, videos and files chosen in the app, or what is shared to it
 * from another app (words too), sent onto that PC's station as nearby transfer sends ([Nearby]). One handing at a time,
 * sent by a service of its own ([HandService]) that goes on when the app is left, its progress in a notification.
 */
object Hand {
    private const val CHANNEL = "hand"
    const val GOING = 7311
    private const val OVER = 7312

    /** How long a PC receiving another sending is waited for (ms), tried again every [AGAIN] ms meanwhile. */
    private const val BUSY_FOR = 120_000L
    private const val AGAIN = 2_000L

    /**
     * One thing given to be sent: where it is (given this app to read, or a file of this phone's any app reads: an app's
     * package), and what the PC is told of it where that is to be other than its provider says (a folder's: by its
     * place in the folder, "folder/inner/name"), with its size and type as they were found.
     */
    class Pick(val uri: Uri, val name: String? = null, val size: Long? = null, val type: String? = null)

    /** One thing handed over: what the PC is told of it, how it is read, and where it was given (to show it). */
    class Handed(val thing: Nearby.Thing, val uri: Uri?) {
        val picture get() = thing.type.startsWith("image/")
        val video get() = thing.type.startsWith("video/")
    }

    /** Why a handing ended short. */
    enum class Why {
        /** The PC was not reached (asleep, or on another network). */
        Unreached,
        /** Broken off on the way. */
        Broken,
        /** The PC kept receiving another sending for longer than it is waited for. */
        Busy,
        /** More than the station on the PC has room for. */
        NoRoom,
        /** A file larger than the PC takes ([Job.named]). */
        TooLarge,
        /** A file that could not be read ([Job.named]). */
        Unread,
        /** The PC's user turned it down. */
        Refused,
        /** The PC does not receive from devices nearby (its settings have it off). */
        Off,
    }

    sealed interface State {
        /** Being read, and the PC reached. */
        data object Reaching : State
        /** The PC is receiving another sending: waited for. */
        data object Waiting : State
        data object Going : State
        data object Sent : State
        data class Failed(val why: Why) : State
        data object Cancelled : State
    }

    /**
     * A handing: to which PC, what (what is known of it yet: nothing while it is read), how it goes, how many bytes of it
     * went (and of each thing, by its id), which things the PC has, the one a failure names, whether its sheet was put
     * away, and the folders what it sends is read through (given leave to read again where it is tried again).
     */
    data class Job(
        val number: Int,
        val pc: Pc,
        val things: List<Handed>,
        val state: State,
        val done: Long = 0,
        val moved: Map<String, Long> = emptyMap(),
        val taken: Set<String> = emptySet(),
        val named: String? = null,
        val away: Boolean = false,
        val trees: List<Uri> = emptyList(),
    ) {
        val total get() = things.sumOf { it.thing.size }
        val over get() = state is State.Sent || state is State.Failed || state is State.Cancelled
        /** What it sends, said: all words (one message), all photos, or files. */
        val words get() = things.size == 1 && things[0].thing.words != null
        val photos get() = things.isNotEmpty() && things.all { it.picture }
        /** What of it the PC does not have yet (what is sent again where it is tried again). */
        val left get() = things.filter { it.thing.id !in taken }
    }

    private val shown = MutableStateFlow<Job?>(null)
    /** The handing going on, or the last one, till it is put away. */
    val now: StateFlow<Job?> = shown

    /**
     * What [HandService] sends next, as it was given: the PC, and the things, or where they are (and the folders they
     * are in, read through them) and words.
     */
    private class Given(val pc: Pc, val picks: List<Pick>, val trees: List<Uri>, val words: String?, val things: List<Handed>?)

    @Volatile
    private var given: Given? = null
    @Volatile
    private var stopped = false
    private var numbers = 0

    private fun kept(context: Context) = context.getSharedPreferences("hand", Context.MODE_PRIVATE)

    /** The PC last handed to, by its key. */
    fun last(context: Context): String? = kept(context).getString("last", null)

    /**
     * [picks] (readable by this app, those of a folder chosen through its tree among [trees]) or [words] handed to [pc]:
     * sent from now on. Whether they are (not while another handing goes on: one at a time).
     */
    fun start(context: Context, pc: Pc, picks: List<Pick>, words: String? = null, trees: List<Uri> = emptyList()): Boolean {
        if (shown.value?.over == false) return false
        // (What an earlier handing read into the cache is no longer wanted: it is not tried again now.)
        File(context.cacheDir, "hand").deleteRecursively()
        return begin(context, Given(pc, picks, trees, words, null))
    }

    /** What of the last handing the PC does not have, sent to it again. */
    fun retry(context: Context) {
        val job = shown.value?.takeIf { it.state is State.Failed } ?: return
        begin(context, Given(job.pc, emptyList(), job.trees, null, job.left))
    }

    private fun begin(context: Context, what: Given): Boolean {
        if (shown.value?.over == false) return false
        stopped = false
        given = what
        kept(context).edit().putString("last", what.pc.key).apply()
        shown.value = Job(++numbers, what.pc, what.things.orEmpty(), State.Reaching, trees = what.trees)
        // (The places given are handed to the service with leave to read them, a folder with leave to read all in it,
        // its things not named one by one: it reads them while it runs. A file of this phone's needs no leave.)
        val intent = Intent(context, HandService::class.java)
        val places = what.things?.mapNotNull { it.uri } ?: what.picks.map { it.uri }
        val granted = what.trees + places.filter { place -> place.scheme == "content" && what.trees.none { place.toString().startsWith("$it/") } }
        if (granted.isNotEmpty()) {
            intent.clipData = ClipData.newRawUri("", granted[0]).apply { granted.drop(1).forEach { addItem(ClipData.Item(it)) } }
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            if (what.trees.isNotEmpty()) intent.addFlags(Intent.FLAG_GRANT_PREFIX_URI_PERMISSION)
        }
        context.startForegroundService(intent)
        return true
    }

    /** The handing going on, cancelled: what the PC has stays there. */
    fun cancel() {
        stopped = true
    }

    /** Its sheet put away: going on, it goes on, told in the notification. */
    fun putAway() {
        shown.value = shown.value?.copy(away = true)
    }

    /** Its sheet shown again (from its notification). */
    fun bringBack() {
        shown.value = shown.value?.copy(away = false)
    }

    private fun change(number: Int, how: (Job) -> Job) {
        synchronized(this) {
            shown.value?.takeIf { it.number == number }?.let { shown.value = how(it) }
        }
    }

    /** Sends what was given ([begin]), as [service], telling how it goes, and then the service ends. */
    fun run(service: HandService) {
        val what = given ?: return service.end()
        val job = shown.value ?: return service.end()
        val number = job.number
        val context = service.applicationContext
        val ended = try {
            val things = what.things ?: read(context, what.picks, what.words)
            change(number) { it.copy(things = things) }
            sendAll(context, what.pc, things, number, service)
        } catch (unread: Unreadable) {
            change(number) { it.copy(named = unread.name) }
            State.Failed(Why.Unread)
        }
        change(number) { it.copy(state = ended) }
        service.ended(shown.value)
    }

    /** One that could not be read, by its name. */
    private class Unreadable(val name: String) : Exception()

    /**
     * What is at [picks] (or [words]) as things to send: each by its name, type and size (as it was found, else as its
     * provider says), read as it is sent.
     */
    private fun read(context: Context, picks: List<Pick>, words: String?): List<Handed> {
        if (words != null) return listOf(Handed(Nearby.Thing.words(UUID.randomUUID().toString(), words), null))
        val resolver = context.contentResolver
        return picks.mapIndexed { at, pick ->
            val uri = pick.uri
            val (name, size) = if (pick.name != null && pick.size != null) {
                pick.name to pick.size
            } else {
                resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { rows ->
                    if (rows.moveToFirst()) rows.getString(0) to (if (rows.isNull(1)) null else rows.getLong(1)) else null to null
                }?.let { (named, sized) -> (pick.name ?: named) to (pick.size ?: sized) } ?: (pick.name to pick.size)
            }
            val shown = name?.takeIf { it.isNotBlank() } ?: uri.lastPathSegment ?: "file"
            val type = pick.type ?: resolver.getType(uri) ?: typeOf(shown)
            val id = "${at}_${UUID.randomUUID()}"
            // Its size as its provider says it, else as its file is. Where neither is known, it is read once into this
            // app's cache, and sent from there.
            val known = size?.takeIf { it >= 0 } ?: try {
                resolver.openAssetFileDescriptor(uri, "r")?.use { it.length }?.takeIf { it != AssetFileDescriptor.UNKNOWN_LENGTH }
            } catch (failed: Exception) {
                throw Unreadable(shown)
            }
            if (known != null) {
                Handed(Nearby.Thing(id, shown, type, known) { resolver.openInputStream(uri) ?: throw IOException("gone") }, uri)
            } else {
                val copy = File(File(context.cacheDir, "hand").apply { mkdirs() }, id)
                try {
                    (resolver.openInputStream(uri) ?: throw IOException("gone")).use { input -> copy.outputStream().use { input.copyTo(it) } }
                } catch (failed: Exception) {
                    copy.delete()
                    throw Unreadable(shown)
                }
                Handed(Nearby.Thing(id, shown, type, copy.length()) { copy.inputStream() }, uri)
            }
        }
    }

    /** [things] sent to [pc] (trusting this phone first), as handing [number] tells: how it ended. */
    private fun sendAll(context: Context, pc: Pc, things: List<Handed>, number: Int, service: HandService): State {
        val at = Nearby.reach(context, pc) ?: return State.Failed(Why.Unreached)
        if (!at.on) return State.Failed(Why.Off)
        // This phone's certificate told to the PC over what they are paired by: what it sends is taken without asking.
        try {
            at.pc.said("/v1/nearby/trust", JSONObject().put("fingerprint", Own.fingerprint()).put("alias", Own.alias(context)))
        } catch (failure: Failure) {
            return State.Failed(Why.Unreached)
        }
        things.firstOrNull { it.thing.size > at.mostFile }?.let { large ->
            change(number) { it.copy(named = large.thing.name) }
            return State.Failed(Why.TooLarge)
        }
        if (things.sumOf { it.thing.size } > at.mostSending) return State.Failed(Why.NoRoom)
        val sizes = things.associate { it.thing.id to it.thing.size }
        // What went of each, as it goes (the PC told of each in turn).
        val moved = HashMap<String, Long>()
        var told = 0L
        for (batch in things.chunked(at.mostFiles)) {
            val since = System.currentTimeMillis()
            while (true) {
                change(number) { it.copy(state = State.Going) }
                val ended = Nearby.send(context, at, batch.map { it.thing }, { stopped }, { thing, done ->
                    moved[thing.id] = done
                    val now = System.currentTimeMillis()
                    if (now - told >= 200) {
                        told = now
                        change(number) { it.copy(done = moved.values.sum(), moved = moved.toMap()) }
                        service.going(shown.value)
                    }
                }) { thing ->
                    moved[thing.id] = sizes.getValue(thing.id)
                    change(number) { it.copy(taken = it.taken + thing.id, done = moved.values.sum(), moved = moved.toMap()) }
                    service.going(shown.value)
                }
                when (ended) {
                    Nearby.Ended.Sent -> break
                    Nearby.Ended.Busy -> {
                        if (System.currentTimeMillis() - since > BUSY_FOR) return State.Failed(Why.Busy)
                        change(number) { it.copy(state = State.Waiting) }
                        service.going(shown.value)
                        Thread.sleep(AGAIN)
                        if (stopped) return State.Cancelled
                    }
                    Nearby.Ended.Refused -> return State.Failed(Why.Refused)
                    Nearby.Ended.NoRoom -> return State.Failed(Why.NoRoom)
                    Nearby.Ended.Broken -> return State.Failed(Why.Broken)
                    Nearby.Ended.Stopped -> return State.Cancelled
                }
            }
        }
        return State.Sent
    }

    // ---- Its notification ----

    private fun channel(context: Context) {
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel(CHANNEL, context.getString(R.string.hand_channel), NotificationManager.IMPORTANCE_LOW))
    }

    /** Opening it brings the handing's sheet back in the app. */
    private fun opening(context: Context) = PendingIntent.getActivity(context, 1, Intent(context, MainActivity::class.java).putExtra(MainActivity.HANDING, true).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)

    /** How [job] goes, as its notification says it while it goes. */
    fun going(context: Context, job: Job?): Notification {
        channel(context)
        val title = when (job?.state) {
            State.Waiting -> context.getString(R.string.hand_waiting, job.pc.name)
            null, State.Reaching -> context.getString(R.string.hand_reaching, job?.pc?.name.orEmpty())
            else -> context.getString(R.string.hand_going, job.pc.name)
        }
        val builder = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_hand)
            .setContentTitle(title)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(opening(context))
        if (job != null && job.state == State.Going && job.total > 0) {
            builder.setContentText(context.getString(R.string.hand_count, job.taken.size, job.things.size, size(job.done), size(job.total)))
            builder.setProgress(1000, (job.done * 1000 / job.total).toInt(), false)
        } else {
            builder.setProgress(0, 0, true)
        }
        return builder.build()
    }

    /** How [job] ended, told where the app is not in front (in front, its sheet says it). */
    fun over(context: Context, job: Job) {
        val notices = context.getSystemService(NotificationManager::class.java)
        if (AutoSend.inFront() || job.state is State.Cancelled || !notices.areNotificationsEnabled()) return
        channel(context)
        val (title, text) = when (val state = job.state) {
            State.Sent -> context.getString(R.string.hand_sent_to, job.pc.name) to said(context, job)
            is State.Failed -> context.getString(R.string.hand_failed_to, job.pc.name) to context.getString(R.string.hand_failed_open)
            else -> return
        }
        val builder = Notification.Builder(context, CHANNEL)
            .setSmallIcon(if (job.state == State.Sent) R.drawable.ic_sent else R.drawable.ic_hand)
            .setContentTitle(title)
            .setContentText(text)
            .setAutoCancel(true)
            .setContentIntent(opening(context))
        if (job.state == State.Sent) builder.setTimeoutAfter(6000)
        notices.notify(OVER, builder.build())
    }

    /** What went where, said as its sheet and notification say it once it is all there. */
    fun said(context: Context, job: Job): String = when {
        job.words && job.things[0].thing.words?.trim()?.let(::isLink) == true -> context.getString(R.string.hand_sent_link, job.pc.name)
        job.words -> context.getString(R.string.hand_sent_words, job.pc.name)
        job.photos -> context.resources.getQuantityString(R.plurals.hand_sent_photos, job.things.size, job.things.size, job.pc.name)
        else -> context.resources.getQuantityString(R.plurals.hand_sent_files, job.things.size, job.things.size, job.pc.name)
    }

    /** The type a file is taken for by its name's extension, where nothing says it. */
    fun typeOf(name: String): String =
        android.webkit.MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase()) ?: "application/octet-stream"

    /** Whether [words] are a web address alone (as the station on the PC takes them: a link). */
    fun isLink(words: String) = (words.startsWith("https://") || words.startsWith("http://")) && words.length > "https://".length && words.none { it.isWhitespace() }

    /** A size, as the PC says sizes: a decimal under ten. */
    fun size(bytes: Long): String {
        val units = listOf("B", "KB", "MB", "GB", "TB")
        var value = bytes.toDouble()
        var unit = 0
        while (value >= 1024 && unit < units.size - 1) {
            value /= 1024
            unit++
        }
        return if (unit == 0) "$bytes B" else if (value < 10) "%.1f %s".format(value, units[unit]) else "%.0f %s".format(value, units[unit])
    }
}

/** What sends a handing ([Hand]), in front of the system (it goes on when the app is left), its notification saying how. */
class HandService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val notice = Hand.going(this, Hand.now.value)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) startForeground(Hand.GOING, notice, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC) else startForeground(Hand.GOING, notice)
        thread(name = "hand") { Hand.run(this) }
        return START_NOT_STICKY
    }

    /** How the handing goes now, its notification changed. */
    fun going(job: Hand.Job?) {
        getSystemService(NotificationManager::class.java).notify(Hand.GOING, Hand.going(this, job))
    }

    /** The handing over: told so where the app is not in front, and the service ends. */
    fun ended(job: Hand.Job?) {
        job?.let { Hand.over(this, it) }
        end()
    }

    fun end() {
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    /** Held in front too long (the system's limit for such a service): cancelled. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        Hand.cancel()
    }
}
