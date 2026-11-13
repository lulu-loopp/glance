package dev.weiyi.glance.remote

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas as Drawing
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.text.Collator
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** What may be handed to a PC, as LocalSend's Android app has them, in its order: its sign, its name, and whether it is new. */
enum class Kind(val sign: String, val title: Int, val new: Boolean) {
    Files(Sign.FILE, R.string.hand_files, false),
    Media(Sign.IMAGE, R.string.hand_media, false),
    Clipboard(Sign.CLIPBOARD, R.string.hand_clipboard, true),
    Text(Sign.LINES, R.string.hand_text, true),
    Folder(Sign.FOLDER, R.string.hand_folder, true),
    Apps(Sign.APPS, R.string.hand_apps, true),
}

/** The type an app's package is sent as. */
private const val PACKAGE = "application/vnd.android.package-archive"

/** The most files of a folder, or of the clipboard, listed before it is sent, the rest counted. */
private const val LISTED = 4

private fun kept(context: Context) = context.getSharedPreferences("hand", Context.MODE_PRIVATE)

/** The kinds chosen once at least (a new one is marked as new till then). */
private fun tried(context: Context): Set<String> = kept(context).getStringSet("tried", null).orEmpty()

private fun tryOut(context: Context, kind: Kind) {
    kept(context).edit().putStringSet("tried", tried(context) + kind.name).apply()
}

/**
 * What is handed to [pc]: chosen among six kinds on a sheet (shown while [asked], [done] told as it is put away). Files
 * and photos are chosen with the system's own pickers and sent at once. Words are typed, the clipboard and a folder
 * (chosen with the system's folder picker) shown before they are sent, apps chosen on a page of their own.
 */
@Composable
fun Handing(pc: Pc, asked: Boolean, done: () -> Unit) {
    val context = LocalContext.current
    var open by remember { mutableStateOf<Kind?>(null) }
    var tree by remember { mutableStateOf<Uri?>(null) }
    val media = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { uris -> if (uris.isNotEmpty()) Hand.start(context, pc, uris.map { Hand.Pick(it) }) }
    val documents = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris -> if (uris.isNotEmpty()) Hand.start(context, pc, uris.map { Hand.Pick(it) }) }
    val folders = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        if (uri != null) {
            tree = uri
            open = Kind.Folder
        }
    }
    if (asked) {
        KindSheet(pc, done) { kind ->
            done()
            tryOut(context, kind)
            when (kind) {
                Kind.Files -> documents.launch(arrayOf("*/*"))
                Kind.Media -> media.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
                Kind.Folder -> folders.launch(null)
                else -> open = kind
            }
        }
    }
    val close = { open = null }
    when (open) {
        Kind.Text -> TextSheet(pc, close)
        Kind.Clipboard -> ClipSheet(pc, close)
        Kind.Folder -> tree?.let { FolderSheet(pc, it, close) }
        Kind.Apps -> AppsPage(pc, close)
        else -> {}
    }
}

/** The six kinds, three to a row: a new one marked till it is chosen once. */
@Composable
private fun KindSheet(pc: Pc, dismiss: () -> Unit, choose: (Kind) -> Unit) {
    val context = LocalContext.current
    val tried = remember { tried(context) }
    Sheet(dismiss) {
        SheetTitle(stringResource(R.string.hand_to, pc.name))
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            for (row in Kind.entries.chunked(3)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    for (kind in row) KindTile(kind, kind.new && kind.name !in tried, Modifier.weight(1f)) { choose(kind) }
                }
            }
        }
        SheetWords { Words(stringResource(R.string.cancel), Dim, Modifier.padding(horizontal = 8.dp), press = dismiss) }
    }
}

@Composable
private fun KindTile(kind: Kind, new: Boolean, modifier: Modifier, press: () -> Unit) {
    Box(modifier.clip(RoundedCornerShape(14.dp)).background(Card2).clickable(role = Role.Button, onClick = press)) {
        Column(Modifier.fillMaxWidth().padding(start = 6.dp, top = 16.dp, end = 6.dp, bottom = 13.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Sign(kind.sign, Ink, size = 22.dp)
            Text(stringResource(kind.title), fontSize = 13.sp, color = Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (new) Text(stringResource(R.string.hand_new), fontSize = 10.sp, color = Accent, modifier = Modifier.align(Alignment.TopEnd).padding(top = 6.dp, end = 8.dp))
    }
}

/** The word that sends, filled with the accent (faded, and not pressed, while there is nothing to send). */
@Composable
private fun Fill(text: String, enabled: Boolean = true, press: () -> Unit) {
    Box(
        Modifier.heightIn(min = 44.dp).alpha(if (enabled) 1f else 0.4f).clip(RoundedCornerShape(12.dp)).background(Accent)
            .clickable(enabled = enabled, role = Role.Button, onClick = press).padding(horizontal = 18.dp),
        contentAlignment = Alignment.Center,
    ) { Text(text, color = Color.White, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, maxLines = 1) }
}

/** What is said under what is shown on a sheet. */
@Composable
private fun Hint(text: String) = Text(text, fontSize = 12.sp, lineHeight = 18.sp, color = Dim, modifier = Modifier.padding(start = 4.dp, end = 4.dp, top = 2.dp))

/** The foot of a sheet that sends: put away, or send. */
@Composable
private fun SendWords(enabled: Boolean, dismiss: () -> Unit, send: () -> Unit) = SheetWords {
    Words(stringResource(R.string.cancel), Dim, Modifier.padding(horizontal = 8.dp), press = dismiss)
    Fill(stringResource(R.string.hand_send), enabled, send)
}

// ---- Words ----

/** Words typed (the keyboard up at once) and sent to [pc] as words, as long as there are any. */
@Composable
private fun TextSheet(pc: Pc, dismiss: () -> Unit) {
    val context = LocalContext.current
    var text by rememberSaveable { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    Sheet(dismiss) {
        SheetTitle(stringResource(R.string.hand_text_to, pc.name))
        BasicTextField(
            text, { text = it },
            Modifier.fillMaxWidth().heightIn(min = 96.dp, max = 240.dp).clip(RoundedCornerShape(12.dp)).background(Card2).padding(horizontal = 12.dp, vertical = 10.dp).focusRequester(focus),
            textStyle = TextStyle(color = Ink, fontSize = 14.sp, lineHeight = 22.sp),
            cursorBrush = SolidColor(Accent),
        )
        val count = text.codePointCount(0, text.length)
        Text(pluralStringResource(R.plurals.hand_text_count, count, count), fontSize = 11.5.sp, color = Dim, textAlign = TextAlign.End, modifier = Modifier.fillMaxWidth().padding(end = 4.dp))
        SendWords(text.isNotBlank(), dismiss) {
            Hand.start(context, pc, emptyList(), text)
            dismiss()
        }
        Box(Modifier.imePadding())
    }
    LaunchedEffect(Unit) {
        focus.requestFocus()
        keyboard?.show()
    }
}

// ---- The clipboard ----

/** What the clipboard held as it was read: words, a picture (copied out of it), files, or nothing. */
private sealed interface Clipped {
    class Words(val text: String) : Clipped
    class Picture(val pick: Hand.Pick, val width: Int, val height: Int, val shown: ImageBitmap?) : Clipped
    class Files(val picks: List<Hand.Pick>) : Clipped
    data object Empty : Clipped
}

/**
 * What [clip] holds, as LocalSend takes a clipboard: its words where it has any, else a picture alone (copied into this
 * app's cache by a name of the moment, as the clipboard may change before it is sent), else its files.
 */
private fun clipped(context: Context, clip: ClipData?): Clipped {
    clip ?: return Clipped.Empty
    val items = (0 until clip.itemCount).map(clip::getItemAt)
    val words = items.mapNotNull { it.text?.toString()?.takeIf(String::isNotEmpty) }
    if (words.isNotEmpty()) return Clipped.Words(words.joinToString("\n"))
    val uris = items.mapNotNull { it.uri }
    if (uris.isEmpty()) return Clipped.Empty
    val resolver = context.contentResolver
    val single = uris.singleOrNull()
    val type = single?.let { resolver.getType(it) ?: clip.description.getMimeType(0) }
    if (single != null && type != null && type.startsWith("image/")) {
        val folder = File(context.cacheDir, "clip").apply { deleteRecursively(); mkdirs() }
        val extension = MimeTypeMap.getSingleton().getExtensionFromMimeType(type) ?: type.substringAfter('/')
        val name = context.getString(R.string.hand_clip_name, SimpleDateFormat("yyyy-MM-dd_HH-mm", Locale.ROOT).format(Date())) + ".$extension"
        val copy = File(folder, name)
        val copied = try {
            resolver.openInputStream(single)?.use { input -> copy.outputStream().use { input.copyTo(it) } } != null
        } catch (unread: Exception) {
            false
        }
        // (A picture that cannot be read is sent as the file it is: its sending says it could not be read.)
        if (copied) {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(copy.path, bounds)
            val sample = BitmapFactory.Options().apply { inSampleSize = (bounds.outWidth / 720).coerceAtLeast(1) }
            val shown = BitmapFactory.decodeFile(copy.path, sample)?.asImageBitmap()
            return Clipped.Picture(Hand.Pick(Uri.fromFile(copy), name, copy.length(), type), bounds.outWidth, bounds.outHeight, shown)
        }
    }
    return Clipped.Files(uris.map { described(context, it) })
}

/** [uri] as a thing to send, by the name, size and type its provider gives it. */
private fun described(context: Context, uri: Uri): Hand.Pick {
    val resolver = context.contentResolver
    val (name, size) = try {
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { rows ->
            if (rows.moveToFirst()) rows.getString(0) to (if (rows.isNull(1)) null else rows.getLong(1)) else null to null
        } ?: (null to null)
    } catch (unread: Exception) {
        null to null
    }
    return Hand.Pick(uri, name, size, resolver.getType(uri))
}

/**
 * The clipboard, read once as this is opened (the system tells that it was read), shown before it is sent to [pc]:
 * its words or link (sent as words), its picture (sent as a file), or its files, with a word that it may hold what is
 * private. Empty, it says so.
 */
@Composable
private fun ClipSheet(pc: Pc, dismiss: () -> Unit) {
    val context = LocalContext.current
    val clipped by produceState<Clipped?>(null) {
        val clip = context.getSystemService(ClipboardManager::class.java).primaryClip
        value = withContext(Dispatchers.IO) { clipped(context, clip) }
    }
    Sheet(dismiss) {
        when (val now = clipped) {
            null -> {
                SheetTitle(stringResource(R.string.hand_clip_title))
                Hint(stringResource(R.string.hand_reading))
                SheetWords { Words(stringResource(R.string.cancel), Dim, Modifier.padding(horizontal = 8.dp), press = dismiss) }
            }
            Clipped.Empty -> {
                Column(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.size(48.dp).clip(CircleShape).background(Card2), contentAlignment = Alignment.Center) { Sign(Sign.CLIPBOARD, Dim, size = 22.dp) }
                    Text(stringResource(R.string.hand_clip_none_title), fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = Ink, textAlign = TextAlign.Center)
                    Text(stringResource(R.string.hand_clip_none_what), fontSize = 13.sp, lineHeight = 20.sp, color = Dim, textAlign = TextAlign.Center)
                }
                SheetWords { Words(stringResource(R.string.ok), Accent, Modifier.padding(horizontal = 8.dp), weight = FontWeight.SemiBold, press = dismiss) }
            }
            is Clipped.Words -> {
                SheetTitle(stringResource(R.string.hand_clip_title))
                val link = Hand.isLink(now.text.trim())
                Peek(if (link) Sign.LINK else Sign.LINES, stringResource(if (link) R.string.hand_clip_link else R.string.hand_text)) {
                    Text(now.text, fontSize = 13.5.sp, lineHeight = 21.sp, color = Ink, maxLines = 4, overflow = TextOverflow.Ellipsis)
                }
                Hint(stringResource(R.string.hand_clip_hint))
                SendWords(true, dismiss) {
                    Hand.start(context, pc, emptyList(), now.text)
                    dismiss()
                }
            }
            is Clipped.Picture -> {
                SheetTitle(stringResource(R.string.hand_clip_title))
                Peek(Sign.IMAGE, stringResource(R.string.hand_clip_picture, now.width, now.height, Hand.size(now.pick.size ?: 0))) {
                    now.shown?.let { Image(it, null, Modifier.fillMaxWidth().height(140.dp).clip(RoundedCornerShape(8.dp)), contentScale = ContentScale.Crop) }
                }
                Hint(stringResource(R.string.hand_clip_as, now.pick.name.orEmpty()))
                SendWords(true, dismiss) {
                    Hand.start(context, pc, listOf(now.pick))
                    dismiss()
                }
            }
            is Clipped.Files -> {
                SheetTitle(stringResource(R.string.hand_clip_title))
                val total = now.picks.sumOf { it.size ?: 0 }
                Peek(Sign.FILE, pluralStringResource(R.plurals.hand_clip_files, now.picks.size, now.picks.size, Hand.size(total))) {
                    Listed(now.picks.map { (it.name ?: it.uri.lastPathSegment.orEmpty()) to it.size })
                }
                Hint(stringResource(R.string.hand_clip_hint))
                SendWords(true, dismiss) {
                    Hand.start(context, pc, now.picks)
                    dismiss()
                }
            }
        }
    }
}

/** What is to be sent, shown in a box: what it is (its sign and a word) above it. */
@Composable
private fun Peek(sign: String, label: String, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Card2).padding(horizontal = 12.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Sign(sign, Dim, size = 14.dp)
            Text(label, fontSize = 12.sp, color = Dim, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        content()
    }
}

/** The first files of a list by their names and sizes, the rest counted. */
@Composable
private fun Listed(files: List<Pair<String, Long?>>) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 4.dp)) {
        for ((name, size) in files.take(LISTED)) {
            Row(Modifier.fillMaxWidth().padding(vertical = 2.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(name, fontSize = 12.sp, lineHeight = 18.sp, color = Dim, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                if (size != null) Text(Hand.size(size), fontSize = 12.sp, lineHeight = 18.sp, color = Dim)
            }
        }
        if (files.size > LISTED) Text(pluralStringResource(R.plurals.hand_more, files.size - LISTED, files.size - LISTED), fontSize = 12.sp, lineHeight = 18.sp, color = Dim, modifier = Modifier.padding(vertical = 2.dp))
    }
}

// ---- A folder ----

/** One in a folder, as its provider lists it. */
private class Child(val id: String, val name: String, val type: String?, val size: Long?)

/** A folder chosen, as it is sent: its name, its files (each named by its place in it), how many folders are in it, and its size. */
private class Folder(val name: String, val files: List<Hand.Pick>, val folders: Int) {
    val size get() = files.sumOf { it.size ?: 0 }
}

/**
 * What is in the folder [tree] (chosen with the system's folder picker, which gave leave to read all in it), all of it,
 * each folder's files before the folders in it, by name: each file named by its place in the folder, under its name.
 */
private fun folder(context: Context, tree: Uri): Folder {
    val resolver = context.contentResolver
    val top = DocumentsContract.getTreeDocumentId(tree)
    val name = resolver.query(DocumentsContract.buildDocumentUriUsingTree(tree, top), arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME), null, null, null)?.use { rows ->
        if (rows.moveToFirst()) rows.getString(0) else null
    } ?: top.substringAfterLast(':').substringAfterLast('/')
    val columns = arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE, DocumentsContract.Document.COLUMN_SIZE)
    val order = Collator.getInstance()
    val files = ArrayList<Hand.Pick>()
    var folders = 0
    val waiting = ArrayDeque(listOf(top to name))
    while (true) {
        val (id, path) = waiting.removeFirstOrNull() ?: break
        val children = resolver.query(DocumentsContract.buildChildDocumentsUriUsingTree(tree, id), columns, null, null, null)?.use { rows ->
            buildList { while (rows.moveToNext()) add(Child(rows.getString(0), rows.getString(1), rows.getString(2), if (rows.isNull(3)) null else rows.getLong(3))) }
        }.orEmpty().sortedWith(compareBy(order) { it.name })
        for (child in children) {
            if (child.type == DocumentsContract.Document.MIME_TYPE_DIR) {
                folders++
                waiting.addLast(child.id to "$path/${child.name}")
            } else {
                files += Hand.Pick(DocumentsContract.buildDocumentUriUsingTree(tree, child.id), "$path/${child.name}", child.size, child.type?.takeIf { it.isNotEmpty() })
            }
        }
    }
    return Folder(name, files, folders)
}

/** The folder [tree], read, shown before it is sent to [pc]: its name, how much is in it, and its first files. */
@Composable
private fun FolderSheet(pc: Pc, tree: Uri, dismiss: () -> Unit) {
    val context = LocalContext.current
    val read by produceState<Folder?>(null, tree) { value = withContext(Dispatchers.IO) { folder(context, tree) } }
    Sheet(dismiss) {
        SheetTitle(stringResource(R.string.hand_folder_to, pc.name))
        val now = read
        if (now == null) {
            Hint(stringResource(R.string.hand_reading))
            SheetWords { Words(stringResource(R.string.cancel), Dim, Modifier.padding(horizontal = 8.dp), press = dismiss) }
        } else {
            FolderRead(pc, tree, now, dismiss)
        }
    }
}

/** A folder read: its name, how much is in it, and its first files, to be sent to [pc]. */
@Composable
private fun FolderRead(pc: Pc, tree: Uri, now: Folder, dismiss: () -> Unit) {
    val context = LocalContext.current
    val counts = listOfNotNull(
        pluralStringResource(R.plurals.hand_folder_files, now.files.size, now.files.size),
        if (now.folders > 0) pluralStringResource(R.plurals.hand_folder_folders, now.folders, now.folders) else null,
        Hand.size(now.size),
    ).joinToString(" · ")
    Line(now.name, counts, Modifier.padding(horizontal = 4.dp), lead = {
        Box(Modifier.size(40.dp).clip(RoundedCornerShape(12.dp)).background(Card2), contentAlignment = Alignment.Center) { Sign(Sign.FOLDER, Ink, size = 20.dp) }
    }) {}
    if (now.files.isNotEmpty()) {
        Rule()
        Listed(now.files.map { it.name.orEmpty().removePrefix("${now.name}/") to it.size })
    }
    SendWords(now.files.isNotEmpty(), dismiss) {
        Hand.start(context, pc, now.files, trees = listOf(tree))
        dismiss()
    }
}

// ---- Apps ----

/** An app on this phone: its package's name, its own name, version and files (one, or the parts it was split into). */
private class App(val id: String, val name: String, val version: String?, val files: List<File>, val system: Boolean) {
    val size = files.sumOf { it.length() }

    /** Its files as they are sent: one package by its name and version, or its parts in a folder so named. */
    fun picks(): List<Hand.Pick> {
        val named = fileName(listOfNotNull(name, version).joinToString(" "))
        return if (files.size == 1) {
            listOf(Hand.Pick(Uri.fromFile(files[0]), "$named.apk", files[0].length(), PACKAGE))
        } else {
            files.map { Hand.Pick(Uri.fromFile(it), "$named/${it.name}", it.length(), PACKAGE) }
        }
    }
}

/** [name] as a file may be named on a PC: what a name may not hold put as "_". */
private fun fileName(name: String) = name.map { if (it < ' ' || it in "\\/:*?\"<>|") '_' else it }.joinToString("").trim().trimEnd('.').ifEmpty { "_" }

/** The apps on this phone, by name. */
private fun apps(context: Context): List<App> {
    val manager = context.packageManager
    val order = Collator.getInstance()
    return manager.getInstalledApplications(0).map { info ->
        val version = try {
            manager.getPackageInfo(info.packageName, 0).versionName
        } catch (gone: Exception) {
            null
        }
        val files = listOf(info.sourceDir) + info.splitSourceDirs.orEmpty()
        App(info.packageName, info.loadLabel(manager).toString(), version, files.map(::File), info.flags and ApplicationInfo.FLAG_SYSTEM != 0)
    }.sortedWith(compareBy(order) { it.name })
}

/** An app's icon, drawn small. */
private fun icon(context: Context, id: String, side: Int): ImageBitmap? = try {
    val drawable = context.packageManager.getApplicationIcon(id)
    val bitmap = Bitmap.createBitmap(side, side, Bitmap.Config.ARGB_8888)
    drawable.setBounds(0, 0, side, side)
    drawable.draw(Drawing(bitmap))
    bitmap.asImageBitmap()
} catch (gone: Exception) {
    null
}

/**
 * The apps on this phone, to choose among and send to [pc] as their packages: found by name, the system's own shown only
 * when asked, each chosen with a box. An app split into parts says so, and all of them are sent.
 */
@Composable
private fun AppsPage(pc: Pc, dismiss: () -> Unit) {
    val context = LocalContext.current
    val all by produceState<List<App>?>(null) { value = withContext(Dispatchers.IO) { apps(context) } }
    var search by rememberSaveable { mutableStateOf("") }
    var system by rememberSaveable { mutableStateOf(false) }
    val chosen = remember { mutableStateListOf<String>() }
    Dialog(onDismissRequest = dismiss, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        Box(Modifier.fillMaxSize().background(Ground).statusBarsPadding().navigationBarsPadding().imePadding()) {
            Column(Modifier.fillMaxSize()) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    SignButton(Sign.BACK, stringResource(R.string.back), tint = Ink, press = dismiss)
                    Text(stringResource(R.string.hand_apps_title), fontSize = 18.sp, fontWeight = FontWeight.SemiBold, color = Ink)
                }
                Row(
                    Modifier.padding(horizontal = 16.dp, vertical = 4.dp).fillMaxWidth().heightIn(min = 44.dp).clip(RoundedCornerShape(12.dp)).background(Card).padding(horizontal = 12.dp),
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    Sign(Sign.SEARCH, Dim, size = 18.dp)
                    Box(Modifier.weight(1f)) {
                        if (search.isEmpty()) Text(stringResource(R.string.hand_apps_search), fontSize = 14.sp, color = Dim)
                        BasicTextField(search, { search = it }, Modifier.fillMaxWidth(), singleLine = true, textStyle = TextStyle(color = Ink, fontSize = 14.sp), cursorBrush = SolidColor(Accent))
                    }
                }
                Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.hand_apps_system), fontSize = 14.sp, color = Dim, modifier = Modifier.weight(1f))
                    Toggle(system, stringResource(R.string.hand_apps_system)) { system = it }
                }
                val found = all
                if (found == null) {
                    Text(stringResource(R.string.hand_reading), fontSize = 13.sp, color = Dim, modifier = Modifier.padding(horizontal = 20.dp, vertical = 12.dp))
                } else {
                    val words = search.trim()
                    val shown = found.filter { (system || !it.system) && (words.isEmpty() || it.name.contains(words, ignoreCase = true) || it.id.contains(words, ignoreCase = true)) }
                    if (shown.isEmpty()) Text(stringResource(R.string.hand_apps_none), fontSize = 13.sp, color = Dim, modifier = Modifier.padding(horizontal = 20.dp, vertical = 12.dp))
                    LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 88.dp)) {
                        items(shown, key = { it.id }) { app ->
                            AppRow(app, app.id in chosen) { if (app.id in chosen) chosen.remove(app.id) else chosen.add(app.id) }
                        }
                    }
                }
            }
            val sending = all.orEmpty().filter { it.id in chosen }
            if (sending.isNotEmpty()) {
                Row(
                    Modifier.align(Alignment.BottomCenter).padding(16.dp).fillMaxWidth().heightIn(min = 52.dp).clip(RoundedCornerShape(14.dp)).background(Accent)
                        .clickable(role = Role.Button) {
                            Hand.start(context, pc, sending.flatMap { it.picks() })
                            dismiss()
                        }.padding(horizontal = 16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(pluralStringResource(R.plurals.hand_apps_send, sending.size, sending.size), color = Color.White, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                    Text(Hand.size(sending.sumOf { it.size }), color = Color.White.copy(alpha = 0.9f), fontSize = 14.sp)
                }
            }
        }
    }
}

/** An app on the list: its icon, name, version and size (and the parts it is split into), and its box. */
@Composable
private fun AppRow(app: App, chosen: Boolean, press: () -> Unit) {
    val context = LocalContext.current
    val side = with(androidx.compose.ui.platform.LocalDensity.current) { 40.dp.roundToPx() }
    val picture by produceState<ImageBitmap?>(null, app.id) { value = withContext(Dispatchers.IO) { icon(context, app.id, side) } }
    Column {
        Row(Modifier.fillMaxWidth().heightIn(min = 60.dp).clickable(role = Role.Checkbox, onClick = press).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(Modifier.size(40.dp).clip(RoundedCornerShape(10.dp)).background(Card2)) { picture?.let { Image(it, null, Modifier.fillMaxSize()) } }
            Column(Modifier.weight(1f)) {
                Text(app.name, fontSize = 15.sp, fontWeight = FontWeight.Medium, color = Ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Row(horizontalArrangement = Arrangement.spacedBy(0.dp)) {
                    Text(listOfNotNull(app.version, Hand.size(app.size)).joinToString(" · "), fontSize = 12.5.sp, color = Dim, maxLines = 1)
                    if (app.files.size > 1) Text(" · " + pluralStringResource(R.plurals.hand_apps_split, app.files.size, app.files.size), fontSize = 12.5.sp, color = Accent, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            Box(
                Modifier.size(22.dp).clip(RoundedCornerShape(6.dp)).background(if (chosen) Accent else Color.Transparent).border(1.5.dp, if (chosen) Accent else Faint, RoundedCornerShape(6.dp)),
                contentAlignment = Alignment.Center,
            ) { if (chosen) Sign(Sign.CHECK, Color.White, size = 15.dp) }
        }
        Rule()
    }
}
