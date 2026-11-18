package dev.weiyi.glance.remote

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.util.Size
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.IntentCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

/** How long a handing all there stays said on its sheet before the sheet goes by itself (ms). */
private const val SAID_FOR = 2000L

/** The most things listed on the sheet, the rest counted. */
private const val LISTED = 4

/** A sheet of the app's over the bottom of the screen, put away by [dismiss] (dragged down, or pressed beside). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun Sheet(dismiss: () -> Unit, content: @Composable () -> Unit) {
    ModalBottomSheet(
        onDismissRequest = dismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = Card,
        contentColor = Ink,
        scrimColor = Color.Black.copy(alpha = 0.5f),
        shape = RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp),
        dragHandle = { Box(Modifier.padding(top = 10.dp, bottom = 6.dp).size(width = 36.dp, height = 4.dp).clip(CircleShape).background(Hair)) },
    ) {
        Column(Modifier.fillMaxWidth().navigationBarsPadding().padding(start = 16.dp, end = 16.dp, bottom = 12.dp).animateContentSize(), verticalArrangement = Arrangement.spacedBy(4.dp)) { content() }
    }
}

/** What a sheet is of, at its top. */
@Composable
fun SheetTitle(text: String) = Text(text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, color = Ink, modifier = Modifier.padding(start = 4.dp, top = 2.dp, bottom = 8.dp))

/** A row to choose on a sheet: its sign in a square, its name and what it is under it, and how it stands at its right. */
@Composable
private fun Choice(sign: String, title: String, detail: String, enabled: Boolean = true, right: @Composable () -> Unit = {}, press: () -> Unit) {
    Line(title, detail, Modifier.clip(RoundedCornerShape(12.dp)).clickable(enabled = enabled, role = Role.Button, onClick = press).padding(horizontal = 4.dp).alpha(if (enabled) 1f else 0.45f), lead = {
        Box(Modifier.size(40.dp).clip(RoundedCornerShape(12.dp)).background(Card2), contentAlignment = Alignment.Center) { Sign(sign, Ink, size = 20.dp) }
    }, right = right)
}

/** What a sheet's part is of, small, with a turning arc beside it while it is still looked for. */
@Composable
private fun SheetPart(text: String, looking: Boolean = false) {
    Row(Modifier.padding(start = 4.dp, top = 6.dp, bottom = 2.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(text, fontSize = 12.sp, color = Dim, fontWeight = FontWeight.Medium)
        if (looking) {
            val turn by rememberInfiniteTransition(label = "looking").animateFloat(0f, 360f, infiniteRepeatable(tween(900, easing = LinearEasing)), label = "turn")
            Canvas(Modifier.size(12.dp)) {
                rotate(turn) { drawArc(Dim, 0f, 270f, false, style = Stroke(1.5.dp.toPx(), cap = StrokeCap.Round)) }
            }
        }
    }
}

/** The words at the foot of a sheet, at its right. */
@Composable
fun SheetWords(content: @Composable () -> Unit) = Row(Modifier.fillMaxWidth().padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp, Alignment.End)) { content() }

/** How a PC on the list to choose among stands: being reached, reached (receiving or not), or not reached. */
private enum class Standing { Looking, Ready, Off, Away }

/** How long the sheet to send from says it is looking before it says nothing is nearby (ms). */
private const val LOOKING_FOR = 4000L

/**
 * Where what is shared is sent ("发送到"): the paired PCs [pcs] first (the one handed to last first, the others as they
 * were paired, each kept in its place as it is reached or not: one not reached is dimmed where it is, so that no row is
 * moved from under the finger), then the devices found nearby (looked for while this is open: this phone announced, and
 * those that answer listed as they do). A device chosen is sent to here, how it goes said beside it ("等待对方接受", how
 * far, "已发送"), its PIN asked for where it wants one. All there, the sheet goes 2 s later.
 */
@Composable
fun SendSheet(pcs: List<Pc>, dismiss: () -> Unit, choosePc: (Pc) -> Unit, chooseDevice: (Found.Device) -> Unit) {
    val context = LocalContext.current
    val last = remember { Hand.last(context) }
    val standing = remember { mutableStateMapOf<String, Standing>() }
    LaunchedEffect(pcs) {
        coroutineScope {
            pcs.map { pc ->
                async(Dispatchers.IO) {
                    val at = Nearby.reach(context, pc)
                    standing[pc.key] = when { at == null -> Standing.Away; at.on -> Standing.Ready; else -> Standing.Off }
                }
            }.awaitAll()
        }
    }
    // Looked for while open: this phone announced, and what answers listed (not the paired PCs: they are above).
    var looked by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        Receiver.announce()
        delay(LOOKING_FOR)
        looked = true
    }
    val heard by Receiver.devices.collectAsState()
    val paired = remember(pcs) { Pcs.fingerprints(context).filterKeys { key -> pcs.any { it.key == key } }.values.map { it.uppercase() }.toSet() }
    val nearby = heard.filter { it.fingerprint.uppercase() !in paired }
    val job by Hand.now.collectAsState()
    // The sending begun here to a device: once all there, the sheet goes a moment later.
    var begun by remember { mutableStateOf<Int?>(null) }
    LaunchedEffect(job?.number, job?.state) {
        val now = job ?: return@LaunchedEffect
        if (now.number == begun && now.state == Hand.State.Sent) {
            delay(SAID_FOR)
            Hand.putAway()
            dismiss()
        }
    }
    val ordered = remember(pcs) { pcs.sortedBy { it.key != last } }
    Sheet(dismiss) {
        SheetTitle(stringResource(R.string.hand_choose))
        if (ordered.isNotEmpty()) SheetPart(stringResource(R.string.send_paired))
        ordered.forEachIndexed { at, pc ->
            if (at > 0) Rule()
            val stands = standing[pc.key] ?: Standing.Looking
            val detail = when (stands) {
                Standing.Looking -> stringResource(R.string.hand_pc_looking)
                Standing.Ready -> stringResource(if (pc.key == last) R.string.hand_pc_last else R.string.hand_pc_ready)
                Standing.Off -> stringResource(R.string.hand_pc_off)
                Standing.Away -> stringResource(R.string.hand_pc_away)
            }
            Choice(Sign.MONITOR, pc.name, detail, enabled = stands == Standing.Ready) { choosePc(pc) }
        }
        SheetPart(stringResource(R.string.send_nearby), looking = true)
        if (nearby.isEmpty()) Text(stringResource(if (looked) R.string.send_none else R.string.send_looking), fontSize = 13.sp, color = Dim, modifier = Modifier.padding(start = 4.dp, top = 4.dp, bottom = 8.dp))
        nearby.forEachIndexed { at, device ->
            if (at > 0) Rule()
            val mine = job?.takeIf { it.to.key == device.fingerprint && it.number == begun }
            // (One at a time: while a sending goes, the others wait, and the one it goes to says how it goes.)
            val going = job?.over == false
            Choice(signOf(device.type), device.alias, deviceTags(device.glance, device.model, device.type), enabled = !going || mine != null, right = { mine?.let { DeviceStanding(it) } }) {
                if (!going) {
                    chooseDevice(device)
                    begun = Hand.now.value?.takeIf { it.to.key == device.fingerprint && !it.over }?.number
                }
            }
        }
        SheetWords { Words(stringResource(R.string.cancel), Dim, Modifier.padding(horizontal = 8.dp), press = dismiss) }
    }
    job?.takeIf { it.number == begun }?.let { PinAsk(it) }
}

/** How a sending to a device found nearby stands, said beside it. */
@Composable
private fun DeviceStanding(job: Hand.Job) {
    when (val state = job.state) {
        Hand.State.Reaching -> StatusWords(stringResource(R.string.send_connecting))
        Hand.State.Asking, is Hand.State.Pin -> StatusWords(stringResource(R.string.send_asking))
        Hand.State.Waiting -> StatusWords(stringResource(R.string.send_waiting))
        Hand.State.Going -> StatusWords("${(job.done * 100 / job.total.coerceAtLeast(1)).coerceIn(0, 100)}%", Accent)
        Hand.State.Sent -> StatusWords(stringResource(R.string.send_sent), Good)
        Hand.State.Cancelled -> {}
        is Hand.State.Failed -> StatusWords(stringResource(if (state.why == Hand.Why.Refused) R.string.send_refused else R.string.send_failed), Bad)
    }
}

/**
 * The handing going on (or the last, till it is put away), on a sheet: what goes, how far, and how it ended. All there,
 * it goes by itself a moment later. Put away while it goes, it goes on, its notification saying how. [gone] is told when
 * it is put away.
 */
@Composable
fun HandSheet(gone: () -> Unit = {}) {
    val context = LocalContext.current
    val job by Hand.now.collectAsState()
    val shown = job?.takeIf { !it.away }
    LaunchedEffect(shown?.number, shown?.state) {
        when (shown?.state) {
            Hand.State.Sent -> {
                delay(SAID_FOR)
                Hand.putAway()
            }
            Hand.State.Cancelled -> Hand.putAway()
            null -> gone()
            else -> {}
        }
    }
    shown ?: return
    Sheet(Hand::putAway) {
        when (val state = shown.state) {
            Hand.State.Sent -> Ended(Sign.CHECK, Good, stringResource(R.string.hand_sent), Hand.said(context, shown)) {}
            is Hand.State.Failed -> {
                val sent = shown.taken.size
                val title = when {
                    state.why == Hand.Why.Off -> stringResource(R.string.hand_off_title, shown.to.name)
                    sent > 0 -> stringResource(R.string.hand_failed_some, sent, shown.things.size - sent)
                    else -> stringResource(R.string.hand_failed, shown.to.name)
                }
                val why = when (state.why) {
                    Hand.Why.Unreached -> stringResource(R.string.hand_why_unreached, shown.to.name)
                    Hand.Why.Broken -> stringResource(R.string.hand_why_broken, shown.to.name)
                    Hand.Why.Busy -> stringResource(R.string.hand_why_busy, shown.to.name)
                    Hand.Why.NoRoom -> stringResource(R.string.hand_why_room, shown.to.name)
                    Hand.Why.TooLarge -> stringResource(R.string.hand_why_large, shown.named.orEmpty())
                    Hand.Why.Unread -> stringResource(R.string.hand_why_unread, shown.named.orEmpty())
                    Hand.Why.Refused -> if (shown.to is Hand.To.Device) stringResource(R.string.hand_why_refused_device, shown.to.name) else stringResource(R.string.hand_why_refused)
                    Hand.Why.Off -> stringResource(R.string.auto_pc_off)
                }
                // (Tried again where trying again may go: not what the PC refused, nor what is too large or unread.)
                val again = state.why in setOf(Hand.Why.Unreached, Hand.Why.Broken, Hand.Why.Busy, Hand.Why.NoRoom, Hand.Why.Off)
                Ended(Sign.ALERT, Bad, title, why) {
                    Words(stringResource(R.string.close), Dim, Modifier.padding(horizontal = 8.dp), press = Hand::putAway)
                    if (again) Words(stringResource(R.string.hand_retry), Accent, Modifier.padding(horizontal = 8.dp), weight = FontWeight.SemiBold) { Hand.retry(context) }
                }
            }
            else -> {
                val title = when (state) {
                    Hand.State.Waiting -> stringResource(R.string.hand_waiting, shown.to.name)
                    Hand.State.Asking -> stringResource(R.string.hand_asking, shown.to.name)
                    is Hand.State.Pin -> stringResource(R.string.pin_title, shown.to.name)
                    Hand.State.Going -> stringResource(R.string.hand_going, shown.to.name)
                    else -> stringResource(R.string.hand_reaching, shown.to.name)
                }
                SheetTitle(title)
                for (handed in shown.things.take(LISTED)) Going(handed, shown)
                if (shown.things.size > LISTED) Text(pluralStringResource(R.plurals.hand_more, shown.things.size - LISTED, shown.things.size - LISTED), fontSize = 12.sp, color = Dim, modifier = Modifier.padding(start = 4.dp, top = 2.dp))
                if (shown.things.isNotEmpty() && !shown.words) {
                    Text(stringResource(R.string.hand_count, shown.taken.size, shown.things.size, Hand.size(shown.done), Hand.size(shown.total)), fontSize = 12.sp, color = Dim, modifier = Modifier.padding(start = 4.dp, top = 4.dp))
                }
                SheetWords { Words(stringResource(R.string.cancel), Dim, Modifier.padding(horizontal = 8.dp), press = Hand::cancel) }
            }
        }
    }
    PinAsk(shown)
}

/** How a handing ended: its sign in a round, what is said of it, and what may be done then. */
@Composable
private fun Ended(sign: String, tint: Color, title: String, text: String, words: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.size(52.dp).clip(CircleShape).background(tint.copy(alpha = 0.14f)), contentAlignment = Alignment.Center) { Sign(sign, tint, size = 26.dp) }
        Text(title, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, color = Ink, textAlign = TextAlign.Center)
        Text(text, fontSize = 13.sp, lineHeight = 20.sp, color = Dim, textAlign = TextAlign.Center, modifier = Modifier.padding(horizontal = 8.dp))
    }
    SheetWords(words)
}

/** One thing as it goes: its picture (or sign), its name and size, and how far it went. */
@Composable
private fun Going(handed: Hand.Handed, job: Hand.Job) {
    val thing = handed.thing
    val far = if (thing.id in job.taken) 1f else (job.moved[thing.id] ?: 0L).toFloat() / thing.size.coerceAtLeast(1)
    Row(Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Thumb(handed)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                val name = thing.words?.trim()?.lineSequence()?.firstOrNull() ?: thing.name
                Text(name, fontSize = 13.sp, color = Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                if (thing.words == null) Text(Hand.size(thing.size), fontSize = 12.sp, color = Dim)
            }
            Box(Modifier.fillMaxWidth().height(5.dp).clip(CircleShape).background(Card2)) {
                Box(Modifier.fillMaxWidth(far.coerceIn(0f, 1f)).height(5.dp).clip(CircleShape).background(Accent))
            }
        }
    }
}

/** A thing's picture, small: the system's own of a photo or a video, else the sign of what it is. */
@Composable
private fun Thumb(handed: Hand.Handed) {
    val context = LocalContext.current
    val picture by produceState<ImageBitmap?>(null, handed.uri) {
        val uri = handed.uri
        value = if (uri != null && (handed.picture || handed.video) && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            withContext(Dispatchers.IO) { runCatching { context.contentResolver.loadThumbnail(uri, Size(144, 144), null).asImageBitmap() }.getOrNull() }
        } else {
            null
        }
    }
    Box(Modifier.size(40.dp).clip(RoundedCornerShape(10.dp)).background(Card2), contentAlignment = Alignment.Center) {
        val shown = picture
        if (shown != null) {
            Image(shown, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        } else {
            val words = handed.thing.words
            val sign = when {
                words != null && Hand.isLink(words.trim()) -> Sign.LINK
                words != null -> Sign.TEXT
                handed.picture || handed.video -> Sign.IMAGE
                else -> Sign.FILE
            }
            Sign(sign, Dim, size = 18.dp)
        }
    }
}

/**
 * What another app shares to Glance (photos, videos, files, or words), handed to a paired PC: the one there is, or
 * the one chosen. Over the app it was shared from, it goes once its sheet is put away, and what is sent goes on.
 */
class ShareActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(SystemBarStyle.dark(android.graphics.Color.TRANSPARENT), SystemBarStyle.dark(android.graphics.Color.TRANSPARENT))
        val (uris, words) = shared(intent)
        setContent {
            MaterialTheme(colorScheme = darkColorScheme(primary = Accent, onPrimary = Color.Black, background = Ground, surface = Card, onBackground = Ink, onSurface = Ink)) {
                Shared(uris, words) { finish() }
            }
        }
    }

    /** What [intent] shares: where its things are, or its words (where it shares no things). */
    private fun shared(intent: Intent): Pair<List<Uri>, String?> {
        val uris = when (intent.action) {
            Intent.ACTION_SEND -> listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))
            Intent.ACTION_SEND_MULTIPLE -> IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
            else -> emptyList()
        }
        val words = if (uris.isEmpty()) intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()?.takeIf { it.isNotBlank() } else null
        return uris to words
    }
}

@Composable
private fun Shared(uris: List<Uri>, words: String?, finish: () -> Unit) {
    val context = LocalContext.current
    val pcs = remember { Pcs.all(context) }
    val picks = remember(uris) { uris.map { Hand.Pick(it) } }
    // Whether a handing to a paired PC was begun (once: not again as the screen turns), and whether another went on then.
    // A device nearby is sent to on the sheet itself, which says how it goes.
    var begun by rememberSaveable { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    when {
        uris.isEmpty() && words == null -> LaunchedEffect(Unit) { finish() }
        busy -> Tell(stringResource(R.string.hand_busy_title), stringResource(R.string.hand_busy_what), stringResource(R.string.close), finish)
        begun -> HandSheet(gone = finish)
        else -> SendSheet(
            pcs, finish,
            choosePc = { pc -> if (Hand.start(context, pc, picks, words)) begun = true else busy = true },
            chooseDevice = { device -> if (!Hand.start(context, Hand.To.Device(device), picks, words)) busy = true },
        )
    }
}
