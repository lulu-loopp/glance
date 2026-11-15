package dev.weiyi.glance.remote

import android.Manifest
import android.app.NotificationManager
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.lifecycle.compose.LifecycleResumeEffect
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale

/** What a card of the settings is of, small above it, and something to press at its right. */
@Composable
private fun CardTitle(text: String, right: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(text, color = Dim, fontSize = 13.sp, fontWeight = FontWeight.Medium, modifier = Modifier.weight(1f))
        right()
    }
}

/** A device's sign, in a rounded square: a PC, a laptop or a phone, as its kind says. */
@Composable
fun DeviceSign(sign: String) {
    Box(Modifier.size(36.dp).clip(RoundedCornerShape(11.dp)).background(Card2), contentAlignment = Alignment.Center) { Sign(sign, Ink, size = 18.dp) }
}

/** The sign of a device of LocalSend's [type] ("mobile", "desktop", ...). */
fun signOf(type: String?) = when (type) {
    "mobile" -> Sign.PHONE
    "laptop" -> Sign.LAPTOP
    else -> Sign.MONITOR
}

/** What a device found nearby is, as its tags say it: a Glance or another LocalSend app, and its system (else its kind). */
@Composable
fun deviceTags(glance: Boolean, model: String?, type: String?): String {
    val kind = when (type) {
        "mobile" -> stringResource(R.string.kind_mobile)
        "desktop", "laptop" -> stringResource(R.string.kind_desktop)
        "web" -> stringResource(R.string.kind_web)
        null -> null
        else -> stringResource(R.string.kind_server)
    }
    return listOfNotNull(if (glance) "Glance" else "LocalSend", model ?: kind).joinToString(" · ")
}

/**
 * The devices this phone knows: the PCs paired (each with what it lets this phone do, and 解除配对), then the devices
 * trusted here (移除), with 添加电脑 at the top.
 */
@Composable
fun DevicesCard(pcs: List<Pc>, chosen: Int, add: () -> Unit, unpair: (Int) -> Unit) {
    val context = LocalContext.current
    val changed by Kept.changed.collectAsState()
    val trusted = remember(changed) { Kept.trusted(context) }
    CardTitle(stringResource(R.string.devices)) { Words(stringResource(R.string.add_pc), Accent, Modifier.padding(end = 8.dp), size = 13f, press = add) }
    Panel {
        Column {
            pcs.forEachIndexed { at, pc ->
                if (at > 0) Rule()
                val detail = stringResource(if (Pcs.controls(context, pc.key)) R.string.device_paired else R.string.device_paired_view)
                Line(pc.name, detail, note = stringResource(R.string.looking_at).takeIf { at == chosen }, lead = { DeviceSign(Sign.MONITOR) }) {
                    Words(stringResource(R.string.unpair), Accent) { unpair(at) }
                }
            }
            trusted.forEachIndexed { at, device ->
                if (pcs.isNotEmpty() || at > 0) Rule()
                Line(device.alias, stringResource(R.string.device_trusted), lead = { DeviceSign(signOf(device.type)) }) {
                    Words(stringResource(R.string.remove), Accent) { Kept.forget(context, device.fingerprint) }
                }
            }
            if (pcs.isEmpty() && trusted.isEmpty()) Text(stringResource(R.string.hand_none_title), color = Dim, fontSize = 13.sp, modifier = Modifier.padding(vertical = 8.dp))
        }
    }
}

/** Whether the app's notifications may be shown. */
private fun noticesOn(context: Context) = context.getSystemService(NotificationManager::class.java).areNotificationsEnabled()

/**
 * Receiving from devices nearby: 后台接收 (turned on, the notifications and then the system's leave to run in the
 * background asked for), what is done with devices not trusted, the name this phone is shown by. And, where
 * receiving cannot do all it should (held in the background, no leave to keep files, notifications off), that, with
 * the way to let it.
 */
@Composable
fun NearbyCard() {
    val context = LocalContext.current
    val changed by Kept.changed.collectAsState()
    val background = remember(changed) { Kept.background(context) }
    val others = remember(changed) { Kept.others(context) }
    val alias = remember(changed) { Own.alias(context) }
    var unheld by remember { mutableStateOf(AutoSend.unheld(context)) }
    var notices by remember { mutableStateOf(noticesOn(context)) }
    var writable by remember { mutableStateOf(Store.writable(context)) }
    var choosing by remember { mutableStateOf(false) }
    var naming by remember { mutableStateOf(false) }
    LifecycleResumeEffect(Unit) {
        unheld = AutoSend.unheld(context)
        notices = noticesOn(context)
        writable = Store.writable(context)
        onPauseOrDispose {}
    }
    val lettingGo = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { unheld = AutoSend.unheld(context) }
    val letGo = { lettingGo.launch(AutoSend.letGo(context)) }
    // Turned on: the notifications asked for (Android 13 on), then the system asked to let the app be in the background.
    val noticing = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        notices = noticesOn(context)
        if (Kept.background(context) && !AutoSend.unheld(context)) letGo()
    }
    val storing = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { writable = Store.writable(context) }
    val askNotices = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED

    CardTitle(stringResource(R.string.near_title))
    Panel {
        Column {
            Line(stringResource(R.string.near_background), stringResource(if (background) R.string.near_background_on else R.string.near_background_off)) {
                Toggle(background, stringResource(R.string.near_background)) { wanted ->
                    Kept.setBackground(context, wanted)
                    if (wanted) {
                        ReceiveService.start(context)
                        if (askNotices) noticing.launch(Manifest.permission.POST_NOTIFICATIONS) else if (!AutoSend.unheld(context)) letGo()
                    } else {
                        ReceiveService.running?.show()
                    }
                }
            }
            if (background && !unheld) {
                Rule()
                Line(stringResource(R.string.auto_held), stringResource(R.string.near_held_what)) { Words(stringResource(R.string.auto_held_allow), Accent, press = letGo) }
            }
            if (!writable) {
                Rule()
                Line(stringResource(R.string.near_storage), stringResource(R.string.near_storage_what)) {
                    Words(stringResource(R.string.auto_held_allow), Accent) { storing.launch(Manifest.permission.WRITE_EXTERNAL_STORAGE) }
                }
            }
            if (!notices) {
                Rule()
                Line(stringResource(R.string.near_notices), stringResource(R.string.near_notices_what)) {
                    Words(stringResource(R.string.auto_held_allow), Accent) {
                        if (askNotices) {
                            noticing.launch(Manifest.permission.POST_NOTIFICATIONS)
                        } else {
                            context.startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName))
                        }
                    }
                }
            }
            Rule()
            Line(stringResource(R.string.near_others), null) {
                Words(stringResource(if (others == Lan.Others.Refuse) R.string.near_others_refuse else R.string.near_others_ask), Dim) { choosing = true }
            }
            Rule()
            Line(stringResource(R.string.near_name), null) {
                Words(alias, Dim, Modifier.widthIn(max = 180.dp)) { naming = true }
            }
        }
    }
    if (choosing) {
        Sheet({ choosing = false }) {
            SheetTitle(stringResource(R.string.near_others))
            for ((value, name) in listOf(Lan.Others.Ask to R.string.near_others_ask, Lan.Others.Refuse to R.string.near_others_refuse)) {
                Row(
                    Modifier.fillMaxWidth().heightIn(min = 52.dp).clip(RoundedCornerShape(12.dp)).clickable(role = Role.RadioButton) {
                        Kept.setOthers(context, value)
                        choosing = false
                    }.padding(horizontal = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(stringResource(name), fontSize = 15.sp, color = Ink, modifier = Modifier.weight(1f))
                    if (others == value) Sign(Sign.CHECK, Accent, size = 20.dp)
                }
            }
        }
    }
    if (naming) Naming(alias) { naming = false }
}

/** This phone's name, written in a box: kept as it is written (nothing, or the phone's own name: called as the phone is). */
@Composable
private fun Naming(now: String, dismiss: () -> Unit) {
    val context = LocalContext.current
    var name by remember { mutableStateOf(now) }
    val focus = remember { FocusRequester() }
    Dialog(onDismissRequest = dismiss) {
        Column(Modifier.widthIn(max = 328.dp).clip(RoundedCornerShape(24.dp)).background(Card2).padding(start = 24.dp, top = 24.dp, end = 12.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.near_name), fontSize = 19.sp, fontWeight = FontWeight.SemiBold, color = Ink)
            Text(stringResource(R.string.near_name_what), fontSize = 13.sp, color = Dim, modifier = Modifier.padding(end = 12.dp))
            BasicTextField(
                name, { name = it.take(64) },
                Modifier.padding(end = 12.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Card).padding(horizontal = 12.dp, vertical = 12.dp).focusRequester(focus),
                singleLine = true, textStyle = TextStyle(color = Ink, fontSize = 15.sp), cursorBrush = SolidColor(Accent),
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                Words(stringResource(R.string.cancel), Dim, Modifier.padding(horizontal = 4.dp).widthIn(min = 56.dp), size = 15f, weight = FontWeight.SemiBold, press = dismiss)
                Words(stringResource(R.string.save), Accent, Modifier.padding(horizontal = 4.dp).widthIn(min = 56.dp), size = 15f, weight = FontWeight.SemiBold) {
                    Kept.setAlias(context, name)
                    // (Devices nearby told the new name at once.)
                    Receiver.announce()
                    dismiss()
                }
            }
        }
    }
    LaunchedEffect(Unit) { focus.requestFocus() }
}

/** When [at] (ms) was, said as a list of what came says it: today's and yesterday's by their time, else by their day. */
@Composable
private fun saidWhen(at: Long): String {
    val time = SimpleDateFormat("HH:mm", Locale.ROOT).format(Date(at))
    val then = Calendar.getInstance().apply { timeInMillis = at }
    val now = Calendar.getInstance()
    val days = (now.get(Calendar.YEAR) - then.get(Calendar.YEAR)) * 366 + now.get(Calendar.DAY_OF_YEAR) - then.get(Calendar.DAY_OF_YEAR)
    return when (days) {
        0 -> stringResource(R.string.today_at, time)
        1 -> stringResource(R.string.yesterday_at, time)
        else -> "${SimpleDateFormat(stringResource(R.string.date_pattern), Locale.getDefault()).format(Date(at))} $time"
    }
}

/** What came lately (the last [Kept.RECENT]): each pressed to open, words to copy. */
@Composable
fun RecentCard() {
    val context = LocalContext.current
    val changed by Kept.changed.collectAsState()
    val recent by produceState(emptyList<Kept.Came>(), changed) { value = withContext(Dispatchers.IO) { Kept.recent(context) } }
    CardTitle(stringResource(R.string.near_recent))
    Panel {
        Column {
            if (recent.isEmpty()) Text(stringResource(R.string.near_recent_none), color = Dim, fontSize = 13.sp, modifier = Modifier.padding(vertical = 8.dp))
            recent.forEachIndexed { at, came ->
                if (at > 0) Rule()
                val words = came.words
                val title = words?.lineSequence()?.firstOrNull()?.trim() ?: came.name
                val what = if (words != null) stringResource(R.string.near_recent_words) else Hand.size(came.size)
                Line(
                    title, stringResource(R.string.near_recent_line, "${saidWhen(came.at)} · $what", came.from),
                    Modifier.clip(RoundedCornerShape(12.dp)).clickable(role = Role.Button) {
                        if (words != null) {
                            Store.copy(context, words)
                        } else {
                            came.uri?.let { uri ->
                                try {
                                    context.startActivity(Store.opening(Uri.parse(uri), came.type))
                                } catch (none: ActivityNotFoundException) {
                                    // (No app on this phone opens this kind: nothing to open it in.)
                                }
                            }
                        }
                    },
                    lead = { CameSign(came) },
                ) {}
            }
        }
    }
}

/** What came, small: its picture where it is one (or a video), else the sign of what it is. */
@Composable
private fun CameSign(came: Kept.Came) {
    val context = LocalContext.current
    val picture by produceState<ImageBitmap?>(null, came.id) {
        value = came.uri?.let { uri -> withContext(Dispatchers.IO) { Store.picture(context, Uri.parse(uri), came.type)?.asImageBitmap() } }
    }
    Box(Modifier.size(36.dp).clip(RoundedCornerShape(10.dp)).background(Card2), contentAlignment = Alignment.Center) {
        val shown = picture
        if (shown != null) {
            Image(shown, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        } else {
            val sign = when {
                came.words != null && Hand.isLink(came.words.trim()) -> Sign.LINK
                came.words != null -> Sign.LINES
                came.type.startsWith("video/") -> Sign.VIDEO
                came.type.startsWith("image/") -> Sign.IMAGE
                else -> Sign.FILE
            }
            Sign(sign, Dim, size = 18.dp)
        }
    }
}

/**
 * The PIN a device asks for as [job] goes to it: written in a box and given (for that offer only), or the sending
 * cancelled. A PIN not right is said so, and another may be given.
 */
@Composable
fun PinAsk(job: Hand.Job) {
    val state = job.state as? Hand.State.Pin ?: return
    var pin by remember(job.number, state.tried) { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    Dialog(onDismissRequest = Hand::cancel) {
        Column(Modifier.widthIn(max = 328.dp).clip(RoundedCornerShape(24.dp)).background(Card2).padding(start = 24.dp, top = 24.dp, end = 12.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.pin_title, job.to.name), fontSize = 19.sp, fontWeight = FontWeight.SemiBold, color = Ink, modifier = Modifier.padding(end = 12.dp))
            Box(Modifier.padding(end = 12.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Card).padding(horizontal = 12.dp, vertical = 12.dp)) {
                if (pin.isEmpty()) Text(stringResource(R.string.pin_hint), color = Faint, fontSize = 15.sp)
                BasicTextField(pin, { pin = it.trim().take(64) }, Modifier.fillMaxWidth().focusRequester(focus), singleLine = true, textStyle = TextStyle(color = Ink, fontSize = 15.sp), cursorBrush = SolidColor(Accent))
            }
            if (state.tried > 0) Text(stringResource(R.string.pin_wrong), color = Bad, fontSize = 13.sp)
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                Words(stringResource(R.string.cancel), Dim, Modifier.padding(horizontal = 4.dp).widthIn(min = 56.dp), size = 15f, weight = FontWeight.SemiBold, press = Hand::cancel)
                Words(stringResource(R.string.hand_send), Accent, Modifier.padding(horizontal = 4.dp).widthIn(min = 56.dp), size = 15f, weight = FontWeight.SemiBold, enabled = pin.isNotEmpty()) { Hand.give(pin) }
            }
        }
    }
    LaunchedEffect(job.number, state.tried) { focus.requestFocus() }
}

/** Words said beside a device on the sheet to send from, on one line. */
@Composable
fun StatusWords(text: String, color: Color = Dim) = Text(text, fontSize = 12.5.sp, color = color, maxLines = 1, overflow = TextOverflow.Ellipsis)
