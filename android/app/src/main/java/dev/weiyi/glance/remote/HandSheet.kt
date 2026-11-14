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

/** A row to choose on a sheet: its sign in a square, its name and what it is under it. */
@Composable
private fun Choice(sign: String, title: String, detail: String, enabled: Boolean = true, press: () -> Unit) {
    Line(title, detail, Modifier.clip(RoundedCornerShape(12.dp)).clickable(enabled = enabled, role = Role.Button, onClick = press).padding(horizontal = 4.dp).alpha(if (enabled) 1f else 0.45f), lead = {
        Box(Modifier.size(40.dp).clip(RoundedCornerShape(12.dp)).background(Card2), contentAlignment = Alignment.Center) { Sign(sign, Ink, size = 20.dp) }
    }) {}
}

/** The words at the foot of a sheet, at its right. */
@Composable
fun SheetWords(content: @Composable () -> Unit) = Row(Modifier.fillMaxWidth().padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp, Alignment.End)) { content() }

/** How a PC on the list to choose among stands: being reached, reached (receiving or not), or not reached. */
private enum class Standing { Looking, Ready, Off, Away }

/** Which of the paired PCs [pcs] is handed to: each reached first (the one handed to last first, those not reached last). */
@Composable
fun PcSheet(pcs: List<Pc>, dismiss: () -> Unit, choose: (Pc) -> Unit) {
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
    val ordered = pcs.sortedWith(compareBy<Pc>({ standing[it.key] == Standing.Away || standing[it.key] == Standing.Off }, { it.key != last }))
    Sheet(dismiss) {
        SheetTitle(stringResource(R.string.hand_choose))
        ordered.forEachIndexed { at, pc ->
            if (at > 0) Rule()
            val stands = standing[pc.key] ?: Standing.Looking
            val detail = when (stands) {
                Standing.Looking -> stringResource(R.string.hand_pc_looking)
                Standing.Ready -> stringResource(if (pc.key == last) R.string.hand_pc_last else R.string.hand_pc_ready)
                Standing.Off -> stringResource(R.string.hand_pc_off)
                Standing.Away -> stringResource(R.string.hand_pc_away)
            }
            Choice(Sign.MONITOR, pc.name, detail, enabled = stands == Standing.Ready) { choose(pc) }
        }
        SheetWords { Words(stringResource(R.string.cancel), Dim, Modifier.padding(horizontal = 8.dp), press = dismiss) }
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
                    state.why == Hand.Why.Off -> stringResource(R.string.hand_off_title, shown.pc.name)
                    sent > 0 -> stringResource(R.string.hand_failed_some, sent, shown.things.size - sent)
                    else -> stringResource(R.string.hand_failed, shown.pc.name)
                }
                val why = when (state.why) {
                    Hand.Why.Unreached -> stringResource(R.string.hand_why_unreached, shown.pc.name)
                    Hand.Why.Broken -> stringResource(R.string.hand_why_broken, shown.pc.name)
                    Hand.Why.Busy -> stringResource(R.string.hand_why_busy, shown.pc.name)
                    Hand.Why.NoRoom -> stringResource(R.string.hand_why_room, shown.pc.name)
                    Hand.Why.TooLarge -> stringResource(R.string.hand_why_large, shown.named.orEmpty())
                    Hand.Why.Unread -> stringResource(R.string.hand_why_unread, shown.named.orEmpty())
                    Hand.Why.Refused -> stringResource(R.string.hand_why_refused)
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
                    Hand.State.Waiting -> stringResource(R.string.hand_waiting, shown.pc.name)
                    Hand.State.Going -> stringResource(R.string.hand_going, shown.pc.name)
                    else -> stringResource(R.string.hand_reaching, shown.pc.name)
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
    var chosen by remember { mutableStateOf(pcs.singleOrNull()) }
    // Whether the handing was begun (once: not again as the screen turns), and whether another went on then.
    var begun by rememberSaveable { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    when {
        pcs.isEmpty() -> Tell(stringResource(R.string.hand_none_title), stringResource(R.string.hand_none_what), stringResource(R.string.close), finish)
        uris.isEmpty() && words == null -> LaunchedEffect(Unit) { finish() }
        busy -> Tell(stringResource(R.string.hand_busy_title), stringResource(R.string.hand_busy_what), stringResource(R.string.close), finish)
        begun -> HandSheet(gone = finish)
        else -> when (val pc = chosen) {
            null -> PcSheet(pcs, finish) { chosen = it }
            else -> LaunchedEffect(pc) {
                if (Hand.start(context, pc, uris.map { Hand.Pick(it) }, words)) begun = true else busy = true
            }
        }
    }
}
