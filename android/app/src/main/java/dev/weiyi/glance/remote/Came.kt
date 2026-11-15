package dev.weiyi.glance.remote

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.MediaMetadataRetriever
import android.media.MediaScannerConnection
import android.media.ThumbnailUtils
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.IBinder
import android.os.StatFs
import android.provider.MediaStore
import android.util.Size
import android.webkit.MimeTypeMap
import android.widget.Toast
import androidx.core.content.FileProvider
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.io.OutputStream
import java.util.concurrent.atomic.AtomicInteger

/**
 * What this phone keeps of receiving from devices nearby: whether it receives in the background, what it does with
 * devices not trusted, the name it is shown by, the devices trusted on it, and what came lately (the last
 * [RECENT]). Changed, [changed] is told, so that what shows it is drawn again.
 */
object Kept {
    /** How many things received are kept listed. */
    const val RECENT = 20

    private fun kept(context: Context) = context.getSharedPreferences("nearby", Context.MODE_PRIVATE)

    private val told = MutableStateFlow(0)
    val changed: StateFlow<Int> = told

    private fun change() {
        told.value++
    }

    /** Whether this phone receives with the app not in front (a service of its own keeps it receiving). Off till turned on. */
    fun background(context: Context) = kept(context).getBoolean("background", false)

    fun setBackground(context: Context, on: Boolean) {
        kept(context).edit().putBoolean("background", on).apply()
        change()
    }

    fun others(context: Context): Lan.Others = if (kept(context).getString("others", "ask") == "refuse") Lan.Others.Refuse else Lan.Others.Ask

    fun setOthers(context: Context, others: Lan.Others) {
        kept(context).edit().putString("others", if (others == Lan.Others.Refuse) "refuse" else "ask").apply()
        change()
    }

    /** The name devices nearby are told, where one was given in the app (else the phone's own: see [Own.alias]). */
    fun alias(context: Context): String? = kept(context).getString("alias", null)?.takeIf { it.isNotBlank() }

    /** Named [name] from now on: nothing, or the phone's own name, and it is called as the phone is. */
    fun setAlias(context: Context, name: String) {
        val shown = Lan.shownName(name)
        kept(context).edit().apply { if (shown.isEmpty() || shown == Own.named(context)) remove("alias") else putString("alias", shown) }.apply()
        change()
    }

    /** A device trusted on this phone: by the certificate it proved, with its name, kind (a Glance or not) and since when (ms). */
    class Trusted(val fingerprint: String, val alias: String, val model: String?, val type: String?, val glance: Boolean, val since: Long)

    fun trusted(context: Context): List<Trusted> {
        val list = runCatching { JSONArray(kept(context).getString("trusted", "[]")) }.getOrElse { JSONArray() }
        return (0 until list.length()).mapNotNull { at ->
            val one = list.optJSONObject(at) ?: return@mapNotNull null
            Trusted(one.optString("fingerprint"), one.optString("alias"), one.optString("model").ifEmpty { null }, one.optString("type").ifEmpty { null }, one.optBoolean("glance"), one.optLong("since"))
                .takeIf { it.fingerprint.isNotEmpty() }
        }
    }

    private fun keepTrusted(context: Context, list: List<Trusted>) {
        val json = JSONArray(list.map { JSONObject().put("fingerprint", it.fingerprint).put("alias", it.alias).put("model", it.model.orEmpty()).put("type", it.type.orEmpty()).put("glance", it.glance).put("since", it.since) })
        kept(context).edit().putString("trusted", json.toString()).apply()
        change()
    }

    /** [sender] trusted from now on, by the certificate it proved (its name kept as it is now, since when as it was). */
    @Synchronized
    fun trust(context: Context, sender: Serving.Sender) {
        if (!sender.verified) return
        val list = trusted(context)
        val since = list.firstOrNull { it.fingerprint.equals(sender.fingerprint, ignoreCase = true) }?.since ?: System.currentTimeMillis()
        keepTrusted(context, list.filterNot { it.fingerprint.equals(sender.fingerprint, ignoreCase = true) } + Trusted(sender.fingerprint, sender.alias, sender.model, sender.type, sender.glance, since))
    }

    @Synchronized
    fun forget(context: Context, fingerprint: String) = keepTrusted(context, trusted(context).filterNot { it.fingerprint == fingerprint })

    /**
     * One thing received: words, or a file (where it is kept, its type, size, and on which shelf), from whom, and
     * when (ms). Known by [id].
     */
    class Came(val id: Long, val words: String?, val name: String, val type: String, val uri: String?, val size: Long, val shelf: String?, val from: String, val at: Long) {
        fun toJson(): JSONObject = JSONObject().put("id", id).put("words", words ?: JSONObject.NULL).put("name", name).put("type", type).put("uri", uri ?: JSONObject.NULL)
            .put("size", size).put("shelf", shelf ?: JSONObject.NULL).put("from", from).put("at", at)

        companion object {
            fun of(json: JSONObject) = Came(
                json.optLong("id"), if (json.isNull("words")) null else json.optString("words"), json.optString("name"), json.optString("type"),
                if (json.isNull("uri")) null else json.optString("uri"), json.optLong("size"), if (json.isNull("shelf")) null else json.optString("shelf"),
                json.optString("from"), json.optLong("at"),
            )
        }
    }

    private fun recentFile(context: Context) = File(context.filesDir, "nearby-recent.json")

    /** What came lately, the latest first. */
    @Synchronized
    fun recent(context: Context): List<Came> {
        val file = recentFile(context)
        if (!file.exists()) return emptyList()
        val list = runCatching { JSONArray(file.readText()) }.getOrElse { JSONArray() }
        return (0 until list.length()).mapNotNull { list.optJSONObject(it)?.let(Came::of) }
    }

    /** [came] listed at the head of what came lately (the oldest beyond [RECENT] let go). */
    @Synchronized
    fun came(context: Context, came: List<Came>) {
        val all = (came.reversed() + recent(context)).take(RECENT)
        recentFile(context).writeText(JSONArray(all.map { it.toJson() }).toString())
        change()
    }

    private val numbers = AtomicInteger((System.currentTimeMillis() % 100_000).toInt())

    /** A number for one thing received, or one notification, not given before in this run. */
    fun number(): Int = numbers.incrementAndGet()
}

/**
 * Where what comes is kept: through MediaStore (Android 10 on: no permission, each file pending till it is whole), or,
 * before, as a file of the shared storage (with leave to write it there) made known to the gallery. Pictures and
 * videos land in the gallery's "Glance", the rest in Download/Glance (see [Lan.place]).
 */
object Store {
    /** What is left free on the phone's storage by receiving (bytes). */
    const val ROOM_LEFT = 512L * 1024 * 1024

    private val modern = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q

    /** Whether files may be kept: always from Android 10 on, before only with leave to write the shared storage. */
    fun writable(context: Context) = modern || context.checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED

    /** How much may be received now (bytes): the storage's free space less [ROOM_LEFT]. */
    @Suppress("DEPRECATION")
    fun room(): Long = (StatFs(Environment.getExternalStorageDirectory().path).availableBytes - ROOM_LEFT).coerceAtLeast(0)

    /** Where a file named [sent] goes, its type told by its extension as the system knows it. */
    fun place(sent: String): Lan.Place = Lan.place(sent) { MimeTypeMap.getSingleton().getMimeTypeFromExtension(it) }

    /** One file being written: where it will be, what it is written through, and (before Android 10) the file itself. */
    class Saving(val uri: Uri, val output: OutputStream, val file: File?, val place: Lan.Place)

    /** A file at [place] begun (pending, where MediaStore keeps it, till [finish]). */
    fun open(context: Context, place: Lan.Place): Saving {
        if (modern) {
            val collection = when (place.shelf) {
                Lan.Shelf.Pictures -> MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
                Lan.Shelf.Movies -> MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
                Lan.Shelf.Download -> MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
            }
            val values = ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, place.name)
                put(MediaStore.MediaColumns.MIME_TYPE, place.type)
                put(MediaStore.MediaColumns.RELATIVE_PATH, place.relative)
                put(MediaStore.MediaColumns.IS_PENDING, 1)
            }
            val resolver = context.contentResolver
            val uri = resolver.insert(collection, values) ?: throw IOException("not made")
            val output = try {
                resolver.openOutputStream(uri, "w") ?: throw IOException("not opened")
            } catch (failed: Exception) {
                resolver.delete(uri, null, null)
                throw IOException("not opened")
            }
            return Saving(uri, output, null, place)
        }
        @Suppress("DEPRECATION")
        val folder = File(Environment.getExternalStoragePublicDirectory(place.shelf.root), place.relative.removePrefix("${place.shelf.root}/")).apply { mkdirs() }
        // (A name taken is not written over: "name (1).ext" and on.)
        val stem = place.name.substringBeforeLast('.').takeIf { place.name.contains('.') && !place.name.startsWith('.') } ?: place.name
        val extension = place.name.removePrefix(stem)
        var file = File(folder, place.name)
        var count = 1
        while (!file.createNewFile()) file = File(folder, "$stem (${count++})$extension")
        return Saving(FileProvider.getUriForFile(context, "${context.packageName}.files", file), FileOutputStream(file), file, place)
    }

    /** [saving] whole: shown to the gallery and every app from now on. */
    fun finish(context: Context, saving: Saving) {
        saving.output.close()
        if (saving.file == null) {
            context.contentResolver.update(saving.uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
        } else {
            MediaScannerConnection.scanFile(context, arrayOf(saving.file.path), arrayOf(saving.place.type), null)
        }
    }

    /** [saving] given up: nothing of it is left. */
    fun abandon(context: Context, saving: Saving) {
        runCatching { saving.output.close() }
        if (saving.file == null) context.contentResolver.delete(saving.uri, null, null) else saving.file.delete()
    }

    /** A received file kept as [uri], made smaller to be seen in a notification (none where it is no picture or video, or cannot be read). */
    fun picture(context: Context, uri: Uri, type: String): Bitmap? {
        if (!type.startsWith("image/") && !type.startsWith("video/")) return null
        return runCatching {
            if (modern) {
                context.contentResolver.loadThumbnail(uri, Size(512, 512), null)
            } else if (type.startsWith("video/")) {
                MediaMetadataRetriever().run {
                    try {
                        setDataSource(context, uri)
                        frameAtTime?.let { frame -> ThumbnailUtils.extractThumbnail(frame, 512, 512 * frame.height / frame.width.coerceAtLeast(1)) }
                    } finally {
                        release()
                    }
                }
            } else {
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                context.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, bounds) }
                val sample = BitmapFactory.Options().apply { inSampleSize = (maxOf(bounds.outWidth, bounds.outHeight) / 512).coerceAtLeast(1) }
                context.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, sample) }
            }
        }.getOrNull()
    }

    /** What opens a received file in the app that shows its kind. */
    fun opening(uri: Uri, type: String): Intent = Intent(Intent.ACTION_VIEW).setDataAndType(uri, type).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)

    /** Words received, copied to the clipboard (said so where the system does not say it itself: before Android 13). */
    fun copy(context: Context, words: String) {
        context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("", words))
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) Toast.makeText(context, R.string.copied, Toast.LENGTH_SHORT).show()
    }
}

/**
 * The notifications of receiving: a device asking to send (拒绝, 接收, and 始终接收 where it proved its certificate),
 * what is coming (its bar, 取消: the receiving service's own notification while it goes), what came (打开, its picture
 * where it is one), words that came (复制), and the quiet one the service keeps while it receives in the background.
 */
object Notices {
    const val SERVICE = 7321
    private const val ASK = 7322
    private const val ASKING = "nearby_ask"
    private const val GOING = "nearby_going"
    private const val CAME = "nearby_came"
    private const val QUIET = "nearby_service"

    private fun channels(context: Context) {
        val notices = context.getSystemService(NotificationManager::class.java)
        notices.createNotificationChannels(
            listOf(
                NotificationChannel(ASKING, context.getString(R.string.near_channel_ask), NotificationManager.IMPORTANCE_HIGH),
                NotificationChannel(GOING, context.getString(R.string.near_channel_going), NotificationManager.IMPORTANCE_LOW),
                NotificationChannel(CAME, context.getString(R.string.near_channel_came), NotificationManager.IMPORTANCE_DEFAULT),
                NotificationChannel(QUIET, context.getString(R.string.near_channel_service), NotificationManager.IMPORTANCE_MIN),
            ),
        )
    }

    /** Whether notifications of [channel] may be shown (all of the app's, and that one, allowed). */
    private fun allowed(context: Context, channel: String): Boolean {
        val notices = context.getSystemService(NotificationManager::class.java)
        channels(context)
        return notices.areNotificationsEnabled() && notices.getNotificationChannel(channel)?.importance != NotificationManager.IMPORTANCE_NONE
    }

    private fun broadcast(context: Context, action: String, number: Int, extras: Intent.() -> Unit = {}) =
        PendingIntent.getBroadcast(context, number, Intent(context, NearbyAction::class.java).setAction(action).apply(extras), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)

    /** The app, opened at its settings (where what came is listed). */
    private fun app(context: Context) = PendingIntent.getActivity(context, 2, Intent(context, MainActivity::class.java).putExtra(MainActivity.SETTINGS, true).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)

    /** What [offer] is, said in a few words: the file and its size, or how many and their size, or that it is words or a link. */
    private fun what(context: Context, offer: Lan.Offer): String {
        val words = offer.words
        return when {
            words != null -> context.getString(if (Hand.isLink(words.trim())) R.string.near_ask_link else R.string.near_ask_words)
            offer.files.size == 1 -> "${Lan.parts(offer.files[0].name).joinToString("/")} · ${Hand.size(offer.total)}"
            else -> context.getString(R.string.near_ask_many, offer.files.size, Hand.size(offer.total))
        }
    }

    /** [asking] asked of this phone's user, for a minute at most: whether it could be (notifications allowed). */
    fun ask(context: Context, asking: Serving.Asking): Boolean {
        if (!allowed(context, ASKING)) return false
        val answer = { take: Boolean, trust: Boolean, code: Int ->
            PendingIntent.getForegroundService(
                context, code, Intent(context, ReceiveService::class.java).setAction(ReceiveService.ANSWER).putExtra("ask", asking.id).putExtra("take", take).putExtra("trust", trust),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        }
        val refuse = broadcast(context, NearbyAction.REFUSE, asking.id) { putExtra("ask", asking.id) }
        val builder = Notification.Builder(context, ASKING)
            .setSmallIcon(R.drawable.ic_receive)
            .setContentTitle(context.getString(R.string.near_ask_title, asking.sender.alias))
            .setContentText(what(context, asking.offer))
            .setCategory(Notification.CATEGORY_STATUS)
            .setTimeoutAfter(Serving.ASK_FOR)
            .setDeleteIntent(refuse)
            .addAction(Notification.Action.Builder(null, context.getString(R.string.near_refuse), refuse).build())
            .addAction(Notification.Action.Builder(null, context.getString(R.string.near_take), answer(true, false, asking.id * 4 + 1)).build())
        if (asking.canTrust) builder.addAction(Notification.Action.Builder(null, context.getString(R.string.near_always), answer(true, true, asking.id * 4 + 2)).build())
        context.getSystemService(NotificationManager::class.java).notify(ASK, builder.build())
        return true
    }

    /** The asking answered, or given up: its notification goes. */
    fun asked(context: Context) = context.getSystemService(NotificationManager::class.java).cancel(ASK)

    /** The receiving service's notification while [session] goes: what comes, from whom, how far, and 取消. */
    fun going(context: Context, session: Serving.Session): Notification {
        channels(context)
        val files = session.files.values
        val title = files.singleOrNull()?.let { context.getString(R.string.near_going_one, it.place.name) } ?: context.getString(R.string.near_going_many, files.size)
        val total = session.total.coerceAtLeast(1)
        return Notification.Builder(context, GOING)
            .setSmallIcon(R.drawable.ic_receive)
            .setContentTitle(title)
            .setContentText(context.getString(R.string.near_going_from, session.sender.alias, Hand.size(session.done), Hand.size(session.total)))
            .setProgress(1000, (session.done * 1000 / total).toInt(), false)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(app(context))
            .addAction(Notification.Action.Builder(null, context.getString(R.string.cancel), broadcast(context, NearbyAction.CANCEL, 3)).build())
            .build()
    }

    /** The receiving service's notification while it only waits for what comes (in the background): quiet. */
    fun quiet(context: Context): Notification {
        channels(context)
        return Notification.Builder(context, QUIET)
            .setSmallIcon(R.drawable.ic_receive)
            .setContentTitle(context.getString(R.string.near_service_title))
            .setContentText(context.getString(R.string.near_service_as, Own.alias(context)))
            .setOngoing(true)
            .setContentIntent(app(context))
            .build()
    }

    /** What came from [from]: the file (opened by 打开, its picture shown where it is one) or how many, and where they are kept. */
    fun came(context: Context, from: String, came: List<Kept.Came>) {
        if (came.isEmpty() || !allowed(context, CAME)) return
        val shelves = came.mapNotNull { it.shelf }.toSet()
        val gallery = shelves.any { it != Lan.Shelf.Download.name }
        val downloads = Lan.Shelf.Download.name in shelves
        val where = context.getString(
            when {
                gallery && downloads -> R.string.near_came_both
                gallery -> R.string.near_came_gallery
                else -> R.string.near_came_downloads
            },
            from,
        )
        val one = came.singleOrNull()
        val number = Kept.number()
        val opening = one?.uri?.let { uri -> PendingIntent.getActivity(context, number, Store.opening(Uri.parse(uri), one.type), PendingIntent.FLAG_IMMUTABLE) } ?: app(context)
        val builder = Notification.Builder(context, CAME)
            .setSmallIcon(R.drawable.ic_receive)
            .setContentTitle(if (one != null) context.getString(R.string.near_came_one, one.name) else context.getString(R.string.near_came_many, came.size))
            .setContentText(where)
            .setAutoCancel(true)
            .setContentIntent(opening)
            .addAction(Notification.Action.Builder(null, context.getString(R.string.near_open), opening).build())
        val picture = one?.uri?.let { Store.picture(context, Uri.parse(it), one.type) }
        if (picture != null) builder.setLargeIcon(picture).setStyle(Notification.BigPictureStyle().bigPicture(picture).bigLargeIcon(null as Bitmap?))
        context.getSystemService(NotificationManager::class.java).notify(number, builder.build())
    }

    /** Words that came from [from]: shown, with 复制. */
    fun words(context: Context, from: String, came: Kept.Came) {
        if (!allowed(context, CAME)) return
        val words = came.words.orEmpty()
        val number = came.id.toInt()
        val notice = Notification.Builder(context, CAME)
            .setSmallIcon(R.drawable.ic_receive)
            .setContentTitle(context.getString(R.string.near_words))
            .setSubText(context.getString(R.string.near_from, from))
            .setContentText(words)
            .setStyle(Notification.BigTextStyle().bigText(words))
            .setAutoCancel(true)
            .setContentIntent(app(context))
            .addAction(Notification.Action.Builder(null, context.getString(R.string.near_copy), broadcast(context, NearbyAction.COPY, number) { putExtra("came", came.id) }).build())
            .build()
        context.getSystemService(NotificationManager::class.java).notify(number, notice)
    }
}

/** What a notification of receiving is answered by, where no service need start: refused, cancelled, copied. */
class NearbyAction : BroadcastReceiver() {
    companion object {
        const val REFUSE = "dev.weiyi.glance.remote.nearby.REFUSE"
        const val CANCEL = "dev.weiyi.glance.remote.nearby.CANCEL"
        const val COPY = "dev.weiyi.glance.remote.nearby.COPY"
    }

    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            REFUSE -> Receiver.answer(intent.getIntExtra("ask", 0), take = false, trust = false)
            CANCEL -> Receiver.cancel()
            COPY -> {
                val id = intent.getLongExtra("came", 0)
                Kept.recent(context).firstOrNull { it.id == id }?.words?.let { Store.copy(context, it) }
                context.getSystemService(NotificationManager::class.java).cancel(id.toInt())
            }
        }
    }
}

/**
 * Receiving kept up in front of the system: while 后台接收 is on (its notification quiet), and while something comes
 * (its notification what comes and how far), so that it goes on when the app is left. It ends once neither is so.
 */
class ReceiveService : Service() {
    companion object {
        /** A device's asking answered from its notification (it starts this, which may go on to receive what it sends). */
        const val ANSWER = "dev.weiyi.glance.remote.nearby.ANSWER"

        @Volatile
        var running: ReceiveService? = null
            private set

        /** Started (from the app in front, or as the phone starts, where 后台接收 is on). */
        fun start(context: Context) = context.startForegroundService(Intent(context, ReceiveService::class.java))
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        running = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val notice = Receiver.going(this)?.let { Notices.going(this, it) } ?: Notices.quiet(this)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) startForeground(Notices.SERVICE, notice, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE) else startForeground(Notices.SERVICE, notice)
        Receiver.carried(applicationContext, true)
        if (intent?.action == ANSWER) Receiver.answer(intent.getIntExtra("ask", 0), intent.getBooleanExtra("take", false), intent.getBooleanExtra("trust", false))
        show()
        return START_STICKY
    }

    /** Its notification as things are now: what comes, else waiting (in the background), else it ends. */
    fun show() {
        val going = Receiver.going(this)
        when {
            going != null -> getSystemService(NotificationManager::class.java).notify(Notices.SERVICE, Notices.going(this, going))
            Kept.background(this) -> getSystemService(NotificationManager::class.java).notify(Notices.SERVICE, Notices.quiet(this))
            // (Asked about, a sending may still come: the service waits for its answer.)
            Receiver.asking() -> {}
            else -> {
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
        }
    }

    override fun onDestroy() {
        running = null
        Receiver.carried(applicationContext, false)
        super.onDestroy()
    }
}
