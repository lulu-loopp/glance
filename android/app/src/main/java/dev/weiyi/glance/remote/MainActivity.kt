package dev.weiyi.glance.remote

import android.content.Intent
import android.os.Bundle
import android.os.SystemClock
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.togetherWith
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.min

/** From this width on (a tablet, a phone unfolded) the PC's picture has the right side to itself. */
private val WIDE = 600.dp

class MainActivity : ComponentActivity() {
    /** A pairing link this activity was opened with, till it is taken. */
    private var opened by mutableStateOf<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Dark behind the system's bars too, their icons light.
        enableEdgeToEdge(SystemBarStyle.dark(android.graphics.Color.TRANSPARENT), SystemBarStyle.dark(android.graphics.Color.TRANSPARENT))
        opened = intent?.dataString
        // New photos sent by themselves: noticed again where they are (an app stopped by its user loses its jobs).
        kotlin.concurrent.thread(name = "auto-send-resume") { AutoSend.resume(applicationContext) }
        setContent {
            MaterialTheme(colorScheme = darkColorScheme(primary = Accent, onPrimary = Color.Black, background = Ground, surface = Card, onBackground = Ink, onSurface = Ink)) {
                Surface(Modifier.fillMaxSize(), color = Ground) { App(opened) { opened = null } }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        opened = intent.dataString
    }
}

/** What is over the main page: nothing, the settings, or the pairing of one more PC. */
private enum class Page { Home, Settings, Pairing }

@Composable
private fun App(opened: String?, taken: () -> Unit) {
    val context = LocalContext.current
    var pcs by remember { mutableStateOf(Pcs.all(context)) }
    var chosen by remember { mutableIntStateOf(Pcs.chosen(context)) }
    var page by remember { mutableStateOf(Page.Home) }
    var shown by remember { mutableStateOf(Look.shown(context)) }
    var colours by remember { mutableStateOf(Look.colours(context)) }
    // A newer app than this one, once the site has said so.
    var newer by remember { mutableStateOf(Newer.known(context)) }
    // How taking it in the app stands (see `Update`).
    var taking by remember { mutableStateOf<Update.State>(Update.State.Offered) }
    val updating = rememberCoroutineScope()
    LaunchedEffect(Unit) {
        withContext(Dispatchers.IO) { Newer.ask(context) }
        newer = Newer.known(context)
    }
    // Asked for by hand in the settings: whether it is being asked, and whether the site answered.
    var checking by remember { mutableStateOf(false) }
    var checked by remember { mutableStateOf<Boolean?>(null) }
    val check = {
        updating.launch {
            checking = true
            checked = withContext(Dispatchers.IO) { Newer.ask(context, now = true) }
            newer = Newer.known(context)
            checking = false
        }
        Unit
    }
    // A code scanned that is no pairing code is said on the pairing page.
    var wrong by remember { mutableStateOf(false) }
    var unpairing by remember { mutableStateOf<Int?>(null) }
    val pair = { pc: Pc ->
        Pcs.add(context, pc)
        pcs = Pcs.all(context)
        chosen = Pcs.chosen(context)
        page = Page.Home
    }
    val take = { text: String -> Pc.fromLink(text)?.let(pair) ?: run { wrong = true; page = Page.Pairing } }
    // A link opened pairs with its PC.
    LaunchedEffect(opened) {
        opened?.let(Pc::fromLink)?.let(pair)
        taken()
    }
    val scanner = rememberLauncherForActivityResult(ScanContract()) { result -> result.contents?.let(take) }
    val prompt = stringResource(R.string.scan_prompt)
    val scan = { scanner.launch(ScanOptions().setDesiredBarcodeFormats(ScanOptions.QR_CODE).setPrompt(prompt).setBeepEnabled(false).setOrientationLocked(false)) }

    Box(Modifier.fillMaxSize().safeDrawingPadding()) {
        Column(Modifier.fillMaxSize()) {
            newer?.let { version ->
                val said = when (val state = taking) {
                    Update.State.Offered -> stringResource(R.string.newer_app, version)
                    Update.State.Allow -> stringResource(R.string.newer_allow)
                    is Update.State.Downloading -> state.percent?.let { stringResource(R.string.newer_downloading, it) } ?: stringResource(R.string.newer_downloading_unsized)
                    Update.State.Installing -> stringResource(R.string.newer_installing)
                    Update.State.Failed -> stringResource(R.string.newer_failed)
                }
                Text(
                    said,
                    color = Color.Black, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).clip(RoundedCornerShape(12.dp)).background(Accent)
                        .clickable(role = Role.Button) {
                            val apk = Newer.apk(context)
                            when {
                                // Not said where it is (an older site): its page, as before.
                                apk == null -> context.startActivity(Intent(Intent.ACTION_VIEW, android.net.Uri.parse(Newer.PAGE)))
                                // Under way: once is enough.
                                taking is Update.State.Downloading -> {}
                                // This app may not install apps: the system's page for letting it, come back from to go on.
                                !Update.mayInstall(context) -> {
                                    taking = Update.State.Allow
                                    context.startActivity(Update.allowing(context))
                                }
                                else -> {
                                    taking = Update.State.Downloading(null)
                                    updating.launch(Dispatchers.IO) { Update.take(context, apk) { state -> taking = state } }
                                }
                            }
                        }.padding(horizontal = 14.dp, vertical = 9.dp),
                )
            }
            Computers(pcs, chosen, choose = { at -> Pcs.choose(context, at); chosen = at }, add = { wrong = false; page = Page.Pairing }, settings = { page = Page.Settings })
            val pc = pcs.getOrNull(chosen)
            if (pc == null) {
                Empty(scan) { wrong = false; page = Page.Pairing }
            } else {
                // Each PC with what was last heard of it, not the one before it.
                key(pc.key) {
                    Control(pc, shown, colours, moved = { host -> Pcs.move(context, pc.key, host); pcs = Pcs.all(context) }) { kept -> Look.keep(context, kept); colours = kept }
                }
            }
        }
        // A page over the main one covers it, and takes what is pressed.
        if (page != Page.Home) {
            BackHandler { page = Page.Home }
            Box(Modifier.fillMaxSize().background(Ground).pointerInput(Unit) {}) {
                when (page) {
                    Page.Pairing -> Pairing(first = pcs.isEmpty(), wrong = wrong, back = { page = Page.Home }, scan = scan) { text -> wrong = false; take(text) }
                    else -> SettingsPage(
                        pcs, chosen, shown, newer, checking, checked, check,
                        back = { page = Page.Home },
                        show = { part, on -> Look.show(context, part, on); shown = Look.shown(context) },
                        add = { wrong = false; page = Page.Pairing },
                        unpair = { unpairing = it },
                    )
                }
            }
        }
    }
    // Which PC is to be unpaired is asked by its name.
    unpairing?.let { at ->
        val which = pcs.getOrNull(at) ?: return@let
        Ask(stringResource(R.string.unpair_title, which.name), listOf(stringResource(R.string.unpair_what)), stringResource(R.string.unpair), stringResource(R.string.cancel), dismiss = { unpairing = null }) {
            val looked = pcs.getOrNull(chosen)
            // (Told if it can be reached: one that cannot keeps this phone's key till it is removed there.)
            kotlin.concurrent.thread { runCatching { which.ask("/v1/unpair", JSONObject()) } }
            AutoSend.unpaired(context, which.key)
            Pcs.remove(context, at)
            pcs = Pcs.all(context)
            // The PC looked at stays the one looked at, unless it was the one unpaired.
            val still = pcs.indexOfFirst { it.key == looked?.key }
            if (still >= 0) Pcs.choose(context, still)
            chosen = Pcs.chosen(context)
        }
    }
}

/** The PCs paired with, in a row: the one looked at lit; one more to add; and the settings. */
@Composable
private fun Computers(pcs: List<Pc>, chosen: Int, choose: (Int) -> Unit, add: () -> Unit, settings: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 6.dp, top = 8.dp).heightIn(min = 44.dp), verticalAlignment = Alignment.CenterVertically) {
        Row(Modifier.weight(1f).horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            pcs.forEachIndexed { at, pc -> Chip(pc.name, at == chosen, ground = Card, lit = Accent) { choose(at) } }
            Chip(stringResource(R.string.add), false, ground = Card, press = add)
        }
        SignButton(Sign.GEAR, stringResource(R.string.settings), press = settings)
    }
}

/** A page's top: the way back, and what the page is. */
@Composable
private fun Top(title: String, back: (() -> Unit)?) {
    Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(start = 6.dp, end = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        if (back != null) SignButton(Sign.BACK, stringResource(R.string.back), tint = Ink, press = back) else Spacer(Modifier.width(6.dp))
        Text(title, fontSize = 20.sp, fontWeight = FontWeight.SemiBold, color = Ink)
    }
}

/** With no PC paired, only how to pair one. */
@Composable
private fun Empty(scan: () -> Unit, link: () -> Unit) {
    Column(Modifier.fillMaxSize().padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Panel(Modifier.widthIn(max = 480.dp)) {
            Column(Modifier.fillMaxWidth().padding(vertical = 16.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(stringResource(R.string.empty_title), fontSize = 17.sp, fontWeight = FontWeight.SemiBold, color = Ink)
                Text(stringResource(R.string.empty_how), color = Dim, fontSize = 14.sp, lineHeight = 21.sp, textAlign = TextAlign.Center)
                Box(Modifier.height(48.dp).clip(RoundedCornerShape(16.dp)).background(Accent).clickable(role = Role.Button, onClick = scan).padding(horizontal = 24.dp), contentAlignment = Alignment.Center) {
                    Text(stringResource(R.string.scan), color = Color.Black, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                }
                Words(stringResource(R.string.use_link), Dim, press = link)
            }
        }
    }
}

/** A PC's code is scanned, or its link pasted. */
@Composable
private fun Pairing(first: Boolean, wrong: Boolean, back: () -> Unit, scan: () -> Unit, take: (String) -> Unit) {
    var link by remember { mutableStateOf("") }
    var edited by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxSize()) {
        Top(stringResource(if (first) R.string.pair_title else R.string.pair_another), back)
        Column(Modifier.widthIn(max = 480.dp).verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 4.dp, end = 16.dp, bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.pair_how), color = Dim, fontSize = 14.sp, lineHeight = 21.sp)
            Box(Modifier.fillMaxWidth().height(52.dp).clip(RoundedCornerShape(16.dp)).background(Accent).clickable(role = Role.Button, onClick = scan), contentAlignment = Alignment.Center) {
                Text(stringResource(R.string.scan), color = Color.Black, fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
            }
            OutlinedTextField(link, { link = it; edited = true }, Modifier.fillMaxWidth().padding(top = 12.dp), label = { Text(stringResource(R.string.paste_link)) }, singleLine = true, isError = wrong && !edited)
            if (wrong && !edited) Text(stringResource(R.string.not_a_code), color = Bad, fontSize = 13.sp)
            Words(stringResource(R.string.pair), Accent, size = 15f, weight = FontWeight.SemiBold, enabled = link.isNotBlank()) { edited = false; take(link) }
        }
    }
}

/** The page behind the gear: every PC paired with, each unpaired by itself, and what the main page shows. */
@Composable
private fun SettingsPage(
    pcs: List<Pc>, chosen: Int, shown: Map<String, Boolean>, newer: String?, checking: Boolean, checked: Boolean?, check: () -> Unit,
    back: () -> Unit, show: (String, Boolean) -> Unit, add: () -> Unit, unpair: (Int) -> Unit,
) {
    var licences by remember { mutableStateOf(false) }
    if (licences) Read(stringResource(R.string.licences), LocalContext.current.resources.openRawResource(R.raw.licences).use { it.readBytes().decodeToString() }, stringResource(R.string.close)) { licences = false }
    Column(Modifier.fillMaxSize()) {
        Top(stringResource(R.string.settings), back)
        // What this is, and what it is built with.
        val about: @Composable () -> Unit = {
            Text(stringResource(R.string.about), color = Dim, fontSize = 13.sp, fontWeight = FontWeight.Medium, modifier = Modifier.padding(start = 16.dp, top = 8.dp))
            Panel {
                Column {
                    // Whether there is a newer app, asked for now: what the site said, or that it could not be asked.
                    val said = when {
                        checking -> stringResource(R.string.checking)
                        newer != null -> stringResource(R.string.newer_is, newer)
                        checked == true -> stringResource(R.string.newest)
                        checked == false -> stringResource(R.string.check_failed)
                        else -> null
                    }
                    Line("Glance", listOfNotNull(stringResource(R.string.version, BuildConfig.VERSION_NAME), said).joinToString(" · ")) {
                        if (!checking) Words(stringResource(R.string.check_update), Accent, press = check)
                    }
                    Rule()
                    Line(stringResource(R.string.licences), stringResource(R.string.licences_what)) { Words(stringResource(R.string.view), Accent) { licences = true } }
                }
            }
        }
        val paired: @Composable () -> Unit = {
            Text(stringResource(R.string.paired_pcs), color = Dim, fontSize = 13.sp, fontWeight = FontWeight.Medium, modifier = Modifier.padding(start = 16.dp, top = 8.dp))
            Panel {
                Column {
                    pcs.forEachIndexed { at, pc ->
                        Line(pc.name, pc.host, note = stringResource(R.string.looking_at).takeIf { at == chosen }) { Words(stringResource(R.string.unpair), Bad) { unpair(at) } }
                        Rule()
                    }
                    Words(stringResource(R.string.add_pc), Accent, Modifier.padding(top = 4.dp), size = 15f, press = add)
                }
            }
        }
        val showing: @Composable () -> Unit = {
            Text(stringResource(R.string.shows), color = Dim, fontSize = 13.sp, fontWeight = FontWeight.Medium, modifier = Modifier.padding(start = 16.dp, top = 8.dp))
            Panel {
                Column {
                    val parts = listOf(
                        Triple("rig", R.string.show_rig, R.string.show_rig_what), Triple("reads", R.string.show_reads, R.string.show_reads_what), Triple("power", R.string.power, R.string.show_power_what), Triple("lights", R.string.show_lights, R.string.show_lights_what),
                        Triple("monitors", R.string.show_monitors, R.string.show_monitors_what), Triple("sound", R.string.show_sound, R.string.show_sound_what), Triple("sleep", R.string.show_sleep, null),
                    )
                    parts.forEachIndexed { at, (part, name, what) ->
                        if (at > 0) Rule()
                        Line(stringResource(name), what?.let { stringResource(it) }) { Toggle(shown[part] != false, stringResource(name)) { show(part, it) } }
                    }
                }
            }
        }
        // New photos sent by themselves to the PC looked at.
        val sending: @Composable () -> Unit = { pcs.getOrNull(chosen)?.let { AutoSendCard(it, pcs) } }
        // Narrow, one under the other; wide (a phone unfolded, a tablet), side by side.
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val gap = Arrangement.spacedBy(12.dp)
            if (maxWidth >= WIDE) {
                Row(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 4.dp, end = 16.dp, bottom = 24.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                    Column(Modifier.weight(1f), verticalArrangement = gap) { paired(); sending(); about() }
                    Column(Modifier.weight(1f), verticalArrangement = gap) { showing() }
                }
            } else {
                Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 4.dp, end = 16.dp, bottom = 24.dp), verticalArrangement = gap) {
                    paired()
                    sending()
                    showing()
                    about()
                }
            }
        }
    }
}

/** How long a PC may go without answering before it is said to be away (ms). */
private const val PATIENCE = 4000L

/** How long the page takes to go from a PC not heard from to one that is, and back (ms). */
private const val WAKING = 600

/** A PC of which nothing is read. */
private val UNREAD = Reads(Read(null, null), Read(null, null), Read(null, null), Read(null, null), null, null)

/** What is waiting to be asked of the PC, in turn. */
private class Asking(val what: String, val path: String, val body: JSONObject)

/** A PC paired with: how it is, and what it may be asked. */
@Composable
private fun Control(pc: Pc, shown: Map<String, Boolean>, colours: List<String>, moved: (String) -> Unit, keep: (List<String>) -> Unit) {
    val scope = rememberCoroutineScope()
    var state by remember { mutableStateOf<PcState?>(null) }
    var failure by remember { mutableStateOf<Failure?>(null) }
    // Whether, unanswered, the phone is on the PC's network (see `Pc.near`): what it may be told differs.
    var near by remember { mutableStateOf<Boolean?>(null) }
    // Since when the PC has not answered a look (none: it answered the last): it is not said to be away till that has gone on a while.
    var silent by remember { mutableStateOf<Long?>(null) }
    var busy by remember { mutableStateOf<String?>(null) }
    val waiting = remember { ArrayDeque<Asking>() }
    // How many things have been asked: what a look answers is stale if one was asked meanwhile.
    val asked = remember { intArrayOf(0) }
    var sleepAsked by remember { mutableStateOf(false) }
    var level by remember { mutableFloatStateOf(0f) }
    var sliding by remember { mutableStateOf(false) }
    var part by remember { mutableStateOf(Part.Cpu) }
    // A colour being made, shown before the PC has it.
    var making by remember { mutableStateOf<String?>(null) }
    val context = LocalContext.current

    suspend fun ask(path: String, body: JSONObject? = null) {
        val before = asked[0]
        val answer = try {
            withContext(Dispatchers.IO) { pc.ask(path, body) }
        } catch (error: Failure) {
            // A look unanswered may be a moment's matter (the phone just woken, the network just joined): it is asked again first.
            if (body == null && error == Failure.Unreachable) {
                val since = silent ?: SystemClock.elapsedRealtime().also { silent = it }
                if (SystemClock.elapsedRealtime() - since < PATIENCE) return
            }
            if (body != null || asked[0] == before) failure = error
            if (error == Failure.Unreachable) near = withContext(Dispatchers.IO) { pc.near() }
            return
        }
        silent = null
        if (body == null && asked[0] != before) return
        state = answer
        failure = null
        if (!sliding) answer.volume?.let { level = it.toFloat() }
    }

    // What is asked shows at once as it will be; the PC's answer then says
    // how it is. Asked while another is on its way, it waits its turn (the
    // same thing asked again takes the place of the one waiting).
    fun act(what: String, path: String, body: JSONObject = JSONObject(), expect: (PcState) -> PcState = { it }) {
        asked[0]++
        state = state?.let(expect)
        waiting.removeAll { it.what == what }
        waiting.addLast(Asking(what, path, body))
        if (busy != null) return
        busy = what
        scope.launch {
            while (true) {
                val next = waiting.removeFirstOrNull() ?: break
                busy = next.what
                ask(next.path, next.body)
            }
            busy = null
        }
    }

    // A PC that keeps the colours is given those made on this phone, once: after those it has already (those a
    // phone starts with, where it has kept none).
    val keeps = state?.keepsColours == true
    LaunchedEffect(pc, keeps) {
        val now = state
        if (now != null && keeps && !Look.given(context, pc.key)) {
            val theirs = now.colours ?: colours.filter { it !in Look.made(context) }
            val all = (theirs + Look.made(context).filter { it !in theirs }).take(24)
            if (all != now.colours) act("colours", "/v1/colours", JSONObject().put("colours", JSONArray(all))) { it.copy(colours = all) }
            Look.give(context, pc.key)
        }
    }

    // How it is, asked while this is on screen.
    LaunchedEffect(pc) {
        while (true) {
            if (busy == null) ask("/v1/state")
            // Not answering where it was: looked for on the network, in case its address has changed.
            if (silent != null) withContext(Dispatchers.IO) { pc.find() }?.takeIf { it != pc.host }?.let(moved)
            // (Sooner while it is not yet said to be away.)
            delay(if (silent != null && failure == null) 400 else 1500)
        }
    }
    // Sleep says what it means first.
    if (sleepAsked) {
        Ask(stringResource(R.string.sleep_title), listOf(stringResource(R.string.sleep_what), stringResource(R.string.sleep_instead)), stringResource(R.string.sleep), stringResource(R.string.cancel), dismiss = { sleepAsked = false }) { act("sleep", "/v1/sleep") }
    }

    val reached = state != null && failure == null
    val now = state
    val night = now?.night == true
    // Not heard from yet, or no longer (what was last heard of it is not how it is now): the PC is drawn as it sleeps,
    // dark and breathing slowly, in the place it has when it is heard from; it wakes there when it answers.
    val asleep = now == null || failure == Failure.Unreachable || failure == Failure.Refused
    val away = animateFloatAsState(if (asleep) 1f else 0f, tween(WAKING), label = "away")
    val breathing by remember { derivedStateOf { away.value > 0f } }
    val breath = if (breathing) rememberInfiniteTransition(label = "asleep").animateFloat(0.6f, 1f, infiniteRepeatable(tween(2600, easing = FastOutSlowInEasing), RepeatMode.Reverse), label = "breath") else remember { mutableFloatStateOf(1f) }
    val light = now?.light?.let { if (making != null) it.copy(color = making!!) else it }
    val working = stringResource(R.string.working)

    val rig: @Composable (Modifier, Boolean) -> Unit = { modifier, fill ->
        Column(modifier.fillMaxWidth().animateContentSize().clip(RoundedCornerShape(20.dp)).background(Card).padding(start = 8.dp, top = 8.dp, end = 8.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            val reads = now?.reads.takeUnless { asleep }
            val names = mapOf(Part.Cpu to stringResource(R.string.cpu), Part.Gpu to stringResource(R.string.gpu), Part.Ram to stringResource(R.string.ram), Part.Disk to stringResource(R.string.disk))
            val notes = reads?.let {
                Notes(
                    Part.entries.associateWith { one ->
                        val read = reads.of(one)
                        Note(figure(read), if (read.temp != null && read.load != null) "${names[one]} · ${read.load}%" else names.getValue(one), if (one == Part.Cpu) reads.cpuFan?.let { stringResource(R.string.cpu_fan, it) }.orEmpty() else "")
                    },
                    reads.caseFan?.let { Note(it.toString(), stringResource(R.string.case_fans), stringResource(R.string.rpm)) },
                )
            }
            // (A laptop's light is the one under its keys: in its colour where it takes one, else white.)
            val keysAlone = now?.laptop == true && now.keys != null
            Rig(
                reads, notes, light?.takeIf { now?.lights == true && !asleep }?.let { if (keysAlone && now?.coloursThere != true) Ink else colour(it.color) }, night || asleep, part,
                (if (fill) Modifier.weight(1f).fillMaxWidth() else Modifier.fillMaxWidth().aspectRatio(320f / 254f)).graphicsLayer { alpha = 1f - away.value * (1f - breath.value) },
                laptop = now?.laptop == true,
            ) { part = it }
            if (!asleep && reads == null) Text(stringResource(R.string.no_reads), color = Faint, fontSize = 12.sp, lineHeight = 17.sp, modifier = Modifier.padding(horizontal = 8.dp))
            // (Asleep, the places of its figures with none in them: where they will be.)
            if ((reads != null || asleep) && shown["reads"] != false) ReadsRow(reads ?: UNREAD, part.takeUnless { asleep }, Modifier.padding(horizontal = 4.dp).graphicsLayer { alpha = 1f - 0.6f * away.value }) { part = it }
        }
    }

    val cards: @Composable () -> Unit = cards@{
        when (val why = failure) {
            null -> {}
            is Failure.Failed -> Panel { Text(why.why, color = Dim, fontSize = 14.sp, lineHeight = 21.sp) }
            Failure.Refused -> Panel { Text(stringResource(R.string.refused), color = Dim, fontSize = 14.sp, lineHeight = 21.sp) }
            else -> Panel { Text(unreachable(near), color = Dim, fontSize = 14.sp, lineHeight = 21.sp) }
        }
        if (now == null) return@cards
        // With the picture turned off, the readings alone.
        val reads = now.reads
        if (shown["rig"] == false && shown["reads"] != false && reads != null) {
            Panel {
                ReadsRow(reads, null) {}
                val fans = listOfNotNull(reads.cpuFan?.let { stringResource(R.string.cpu_fan_line, it) }, reads.caseFan?.let { stringResource(R.string.case_fan_line, it) })
                if (fans.isNotEmpty()) Text(fans.joinToString("　"), color = Dim, fontSize = 12.5.sp)
            }
        }
        Tile(
            Sign.NIGHT, stringResource(R.string.night), if (busy == "night") working else stringResource(if (night) R.string.night_on else R.string.night_how),
            lit = night, enabled = reached, big = true, modifier = Modifier.fillMaxWidth(),
        ) { act("night", "/v1/night", on(!night)) { it.copy(night = !night, screen = night, lights = night) } }
        Row(Modifier.height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            // Lit while the screens are on: pressed, they go off, and on again.
            val screen = now.screen
            Tile(
                Sign.SCREEN, stringResource(R.string.screen), if (busy == "screen") working else stringResource(if (screen == false) R.string.screen_is_off else R.string.screen_is_on),
                lit = screen != false, enabled = reached, modifier = Modifier.weight(1f).fillMaxHeight(),
            ) { act("screen", "/v1/screen", on(screen == false)) { it.copy(screen = screen == false) } }
            val lights = now.lights
            // (A PC with no lights to set has no such tile: the screens' then has the row.)
            if (now.lightsThere) Tile(
                Sign.LIGHTS, stringResource(if (now.laptop && now.keys != null) R.string.keys else R.string.lights),
                if (busy == "lights") working else when (lights) { true -> if (now.coloursThere) stringResource(R.string.lights_on, effectName(now.light.effect)) else stringResource(R.string.lights_plain_on); false -> stringResource(R.string.lights_off); null -> stringResource(R.string.lights_unknown) },
                lit = lights != false, enabled = reached, modifier = Modifier.weight(1f).fillMaxHeight(),
            ) { act("lights", "/v1/lights", on(lights == false)) { it.copy(lights = lights == false) } }
        }
        // The PC's power plan, as it is and to choose: what night changes of it is seen here.
        if (shown["power"] != false && now.power.size > 1) {
            Panel {
                Heading(stringResource(R.string.power))
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    for (plan in now.power) {
                        // A power mode is named here; a plan, as Windows names it.
                        val name = when (plan.id) { "efficiency" -> stringResource(R.string.mode_efficiency); "balanced" -> stringResource(R.string.mode_balanced); "performance" -> stringResource(R.string.mode_performance); else -> plan.name }
                        Chip(name, plan.on) { if (reached && !plan.on) act("power", "/v1/power", JSONObject().put("plan", plan.id)) { was -> was.copy(power = was.power.map { it.copy(on = it.id == plan.id) }) } }
                    }
                }
            }
        }
        if (shown["lights"] != false && now.coloursThere && now.lights == true && !night && light != null) {
            // The colours to choose from: the PC's, where it keeps them (the same on every phone and in its own
            // settings); this phone's own, with a PC that does not, or has kept none yet.
            val palette = now.colours ?: colours
            val keepColours: (List<String>) -> Unit = { kept ->
                if (now.keepsColours) {
                    act("colours", "/v1/colours", JSONObject().put("colours", JSONArray(kept))) { it.copy(colours = kept) }
                } else {
                    keep(kept)
                }
            }
            LightsCard(
                light, heat = reads != null, own = palette, enabled = reached, keep = keepColours, preview = { making = it },
                choose = { color, effect, brightness ->
                    making = null
                    val body = JSONObject().put("on", true)
                    color?.let { body.put("color", it) }
                    effect?.let { body.put("effect", it) }
                    brightness?.let { body.put("brightness", it) }
                    act("light", "/v1/lights", body) { it.copy(light = Light(color ?: it.light.color, effect ?: it.light.effect, brightness ?: it.light.brightness)) }
                },
            )
        }
        // A laptop's keys: how bright the light under them is, with the lights.
        val keys = now.keys
        if (shown["lights"] != false && now.lights != false && !night && keys != null) {
            Panel {
                Heading(stringResource(if (now.laptop) R.string.keys_level else R.string.keys))
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    // Out, then each step up to the brightest: named where there are the usual three, numbered otherwise.
                    val names = if (keys.most == 3) listOf(R.string.keys_off, R.string.keys_low, R.string.keys_medium, R.string.keys_high).map { stringResource(it) } else listOf(stringResource(R.string.keys_off)) + (1..keys.most).map { it.toString() }
                    names.forEachIndexed { level, name ->
                        Chip(name, keys.level == level) { if (reached) act("keys", "/v1/keys", JSONObject().put("level", level)) { it.copy(keys = keys.copy(level = level)) } }
                    }
                }
            }
        }
        // Each monitor by itself, where there are several; where there is one that says how bright it is (a laptop's
        // own screen), how bright it is alone (the screens' tile turns it off).
        val alone = now.monitors.singleOrNull()?.takeIf { it.brightness != null }
        if (shown["monitors"] != false && alone != null) {
            Panel {
                Heading(stringResource(R.string.screen_brightness))
                Brightness(alone, enabled = reached && now.screen != false && alone.on != false) { wanted ->
                    act("bright0", "/v1/monitor", JSONObject().put("index", 0).put("brightness", wanted)) { was -> was.copy(monitors = was.monitors.mapIndexed { i, one -> if (i == 0) one.copy(brightness = wanted) else one }) }
                }
            }
        }
        if (shown["monitors"] != false && now.monitors.size > 1) {
            Panel {
                Heading(stringResource(R.string.monitors)) { Places(now.monitors, now.screen != false) }
                Column {
                    now.monitors.forEachIndexed { at, monitor ->
                        if (at > 0) Rule()
                        MonitorRow(
                            at, monitor, enabled = reached && now.screen != false, lit = now.screen != false,
                            turn = { wanted -> act("monitor$at", "/v1/monitor", JSONObject().put("index", at).put("on", wanted)) { was -> was.copy(monitors = was.monitors.mapIndexed { i, one -> if (i == at) one.copy(on = wanted) else one }) } },
                            dim = { wanted -> act("bright$at", "/v1/monitor", JSONObject().put("index", at).put("brightness", wanted)) { was -> was.copy(monitors = was.monitors.mapIndexed { i, one -> if (i == at) one.copy(brightness = wanted) else one }) } },
                        )
                    }
                }
            }
        }
        // The sound: how loud, and what is playing, paused and played from here.
        if (shown["sound"] != false && now.volume != null) {
            Panel {
                val muted = now.muted == true
                Heading(stringResource(R.string.sound)) {
                    Words(stringResource(if (muted) R.string.unmute else R.string.mute), if (muted) Accent else Dim, size = 13f, enabled = reached) { act("mute", "/v1/volume", JSONObject().put("muted", !muted)) { it.copy(muted = !muted) } }
                }
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Bar(
                        level, 0f..100f, stringResource(R.string.volume), Modifier.weight(1f), enabled = reached, track = filled(if (muted) Faint else Accent),
                        change = { sliding = true; level = it },
                        done = { sliding = false; act("volume", "/v1/volume", JSONObject().put("level", level.toInt())) { it.copy(volume = level.toInt(), muted = false) } },
                    )
                    Text(if (muted) stringResource(R.string.muted) else "${level.toInt()}%", color = Ink, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.End, modifier = Modifier.width(44.dp))
                }
                now.media?.let { media ->
                    Rule()
                    if (media.title.isBlank()) Text(stringResource(R.string.media_untold), color = Faint, fontSize = 12.sp, lineHeight = 17.sp)
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Column(Modifier.weight(1f)) {
                            Text(media.title.ifBlank { media.app }, color = Ink, fontSize = 15.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            val about = listOf(stringResource(if (media.playing) R.string.playing else R.string.paused), media.artist, media.app.takeIf { media.title.isNotBlank() }.orEmpty()).filter { it.isNotBlank() }
                            Text(about.joinToString(" · "), color = Dim, fontSize = 12.5.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                        Row {
                            val tell = { what: String, expect: (PcState) -> PcState -> act("media", "/v1/media", JSONObject().put("do", what), expect) }
                            SignButton(Sign.PREVIOUS, stringResource(R.string.previous), tint = Ink, enabled = reached) { tell("previous") { it } }
                            SignButton(if (media.playing) Sign.PAUSE else Sign.PLAY, stringResource(if (media.playing) R.string.pause else R.string.play), tint = Ink, ground = Card2, enabled = reached) {
                                tell("toggle") { it.copy(media = it.media?.copy(playing = !media.playing)) }
                            }
                            SignButton(Sign.NEXT, stringResource(R.string.next), tint = Ink, enabled = reached) { tell("next") { it } }
                        }
                    }
                }
            }
        }
        // Sleep says what it is where it is pressed, and again before it is done.
        if (shown["sleep"] != false) {
            Tile(Sign.SLEEP, stringResource(R.string.sleep), stringResource(R.string.sleep_how), lit = false, enabled = reached, big = true, modifier = Modifier.fillMaxWidth()) { sleepAsked = true }
        }
    }

    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(now?.name?.ifBlank { pc.name } ?: pc.name, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, color = Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                val dot by animateColorAsState(if (reached) Good else if (failure == null) Faint else Bad, tween(WAKING), label = "dot")
                Box(Modifier.size(8.dp).clip(CircleShape).background(dot))
                Text(
                    stringResource(when (failure) { null -> if (now == null) R.string.connecting else R.string.connected; Failure.Unreachable -> R.string.unreachable_short; Failure.Refused -> R.string.refused_short; else -> R.string.failed_short }),
                    color = Dim, fontSize = 13.sp, maxLines = 1,
                )
            }
        }
        // Folded, the PC's picture leads the one column; unfolded, it has the right side to itself.
        BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) {
            // (Asleep, it is pictured whatever is chosen: there is nothing else of it to show.)
            val pictured = shown["rig"] != false || asleep
            val gap = Arrangement.spacedBy(12.dp)
            val rise = with(LocalDensity.current) { 20.dp.roundToPx() }
            // Under the picture, what the PC may be asked; asleep, why that may be. The one comes up over the other as it goes.
            val rest: @Composable () -> Unit = {
                AnimatedContent(asleep, transitionSpec = { (fadeIn(tween(WAKING)) + slideInVertically(tween(WAKING)) { rise }) togetherWith fadeOut(tween(WAKING / 3)) }, label = "rest") { sleeping ->
                    if (sleeping) Away(pc.name, failure, near) else Column(Modifier.fillMaxWidth(), verticalArrangement = gap) { cards() }
                }
            }
            if (maxWidth >= WIDE && pictured) {
                Row(Modifier.fillMaxSize()) {
                    Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 4.dp, end = 16.dp, bottom = 24.dp)) { rest() }
                    Box(Modifier.weight(1f).fillMaxHeight().padding(top = 4.dp, end = 16.dp, bottom = 24.dp)) { rig(Modifier.fillMaxHeight(), true) }
                }
            } else {
                Column(Modifier.widthIn(max = 560.dp).verticalScroll(rememberScrollState()).padding(start = 16.dp, top = 4.dp, end = 16.dp, bottom = 24.dp), verticalArrangement = gap) {
                    if (pictured) rig(Modifier, false)
                    rest()
                }
            }
        }
    }
}

/**
 * One monitor: its number and name, its size and how bright it is, its
 * switch; and under them, where it says how bright it is, that as a line
 * to slide (the figure beside its size follows the finger).
 */
@Composable
private fun MonitorRow(at: Int, monitor: Monitor, enabled: Boolean, lit: Boolean, turn: (Boolean) -> Unit, dim: (Int) -> Unit) {
    val size = "${monitor.width} × ${monitor.height}"
    val detail = when {
        monitor.on == null -> "$size · ${stringResource(R.string.monitor_fixed)}"
        monitor.brightness != null -> "$size · ${stringResource(R.string.brightness_is, monitor.brightness)}"
        else -> size
    }
    Line(
        monitor.name, detail,
        lead = { Box(Modifier.size(24.dp).clip(RoundedCornerShape(7.dp)).background(Card2), contentAlignment = Alignment.Center) { Text("${at + 1}", color = Dim, fontSize = 12.sp, fontWeight = FontWeight.SemiBold) } },
    ) { Toggle(monitor.on != false && lit, monitor.name, enabled = enabled && monitor.on != null, change = turn) }
    if (monitor.brightness != null) Brightness(monitor, enabled && monitor.on != false, dim)
}

/**
 * How bright a monitor is, as a line to slide: as the monitor says, but for while it is being slid (the figure
 * follows the finger), and set as it is let go.
 */
@Composable
private fun Brightness(monitor: Monitor, enabled: Boolean, dim: (Int) -> Unit) {
    var level by remember { mutableFloatStateOf((monitor.brightness ?: 0).toFloat()) }
    var sliding by remember { mutableStateOf(false) }
    LaunchedEffect(monitor.brightness) { if (!sliding) monitor.brightness?.let { level = it.toFloat() } }
    Row(Modifier.fillMaxWidth().padding(bottom = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Box(Modifier.width(24.dp), contentAlignment = Alignment.Center) { Sign(Sign.BRIGHT, if (enabled) Dim else Faint, size = 18.dp) }
        Bar(level, 0f..100f, stringResource(R.string.brightness), Modifier.weight(1f), enabled = enabled, slim = true, track = filled(Accent), change = { sliding = true; level = it }, done = { sliding = false; dim(level.toInt()) })
        Text("${level.toInt()}%", color = Dim, fontSize = 13.sp)
    }
}

/**
 * What may keep a PC from answering: with the phone on its network, it is off or asleep, or the network keeps its
 * devices apart (as campus, hotel and guest networks do), which the PC's own hotspot gets round; with the phone on
 * another, that; and where that cannot be told, either.
 */
@Composable
private fun unreachable(near: Boolean?) = stringResource(when (near) { true -> R.string.unreachable_near; false -> R.string.unreachable_far; null -> R.string.unreachable })

/** Of a PC that has not answered, what may be the matter (nothing yet: it is still being asked). */
@Composable
private fun Away(name: String, failure: Failure?, near: Boolean?) {
    Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
        if (failure != null) {
            Column(Modifier.widthIn(max = 420.dp).padding(top = 8.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(stringResource(if (failure == Failure.Refused) R.string.away_refused_title else R.string.away_title, name), color = Ink, fontSize = 19.sp, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center)
                val what = when (failure) { Failure.Refused -> stringResource(R.string.refused); is Failure.Failed -> failure.why; else -> unreachable(near) }
                Text(what, color = Dim, fontSize = 14.sp, lineHeight = 22.sp, textAlign = TextAlign.Center)
                if (failure == Failure.Unreachable) Text(stringResource(R.string.away_retry), color = Faint, fontSize = 12.5.sp, textAlign = TextAlign.Center)
            }
        }
    }
}

/** A part's one figure: how warm it is; where that is not known, how busy. */
private fun figure(read: Read) = read.temp?.let { "$it°" } ?: read.load?.let { "$it%" } ?: "—"

/** The four parts, each a figure and its name; one pressed is chosen in the picture ([chosen] none: there is no picture). */
@Composable
private fun ReadsRow(reads: Reads, chosen: Part?, modifier: Modifier = Modifier, choose: (Part) -> Unit) {
    val names = mapOf(Part.Cpu to R.string.cpu, Part.Gpu to R.string.gpu, Part.Ram to R.string.ram, Part.Disk to R.string.disk)
    Row(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        for (part in Part.entries) {
            val read = reads.of(part)
            val name = stringResource(names.getValue(part))
            Column(
                Modifier.weight(1f).heightIn(min = 52.dp).clip(RoundedCornerShape(12.dp)).background(Card2)
                    .border(1.dp, if (chosen == part) Accent else Color.Transparent, RoundedCornerShape(12.dp))
                    .clickable(enabled = chosen != null, role = Role.Button) { choose(part) }.padding(horizontal = 8.dp, vertical = 7.dp),
            ) {
                Text(figure(read), color = Ink, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, maxLines = 1)
                Text(if (read.temp != null && read.load != null) "$name ${read.load}%" else name, color = Dim, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
    }
}

/** Where each monitor is, small, to tell them apart: not for pressing. */
@Composable
private fun Places(monitors: List<Monitor>, screen: Boolean) {
    val paint = remember { android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply { textAlign = android.graphics.Paint.Align.CENTER; typeface = android.graphics.Typeface.DEFAULT_BOLD } }
    Canvas(Modifier.size(74.dp, 30.dp)) {
        val left = monitors.minOf { it.x }
        val top = monitors.minOf { it.y }
        val wide = monitors.maxOf { it.x + it.width } - left
        val tall = monitors.maxOf { it.y + it.height } - top
        val line = 1.dp.toPx()
        val k = min((size.width - line) / wide, (size.height - line) / tall)
        // At the card's right edge, in the middle of its own height.
        val origin = Offset(size.width - line / 2 - wide * k, (size.height - tall * k) / 2)
        monitors.forEachIndexed { at, monitor ->
            val on = monitor.on != false && screen
            val corner = Offset(origin.x + (monitor.x - left) * k, origin.y + (monitor.y - top) * k)
            val box = Size(monitor.width * k, monitor.height * k)
            // (A hair apart, where two touch.)
            val inset = Offset(line, line)
            val within = Size(box.width - 2 * line, box.height - 2 * line)
            if (on) drawRoundRect(Lit, corner + inset, within, CornerRadius(2.dp.toPx()))
            drawRoundRect(if (on) Accent else Color(0xFF3A414C), corner + inset, within, CornerRadius(2.dp.toPx()), style = Stroke(line))
            paint.textSize = min(box.height * 0.5f, 8.dp.toPx())
            paint.color = (if (on) Ink else Faint).toArgb()
            drawIntoCanvas { it.nativeCanvas.drawText("${at + 1}", corner.x + box.width / 2, corner.y + box.height / 2 + paint.textSize * 0.36f, paint) }
        }
    }
}
