package dev.weiyi.glance.remote

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.LifecycleResumeEffect
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * New photos sent by themselves to [pc] (one of [pcs], those paired): turned on, the photos asked for (and the notice),
 * the PC asked whether it receives and told this phone's certificate, then the system asked to let the app be in the
 * background; then what else is sent, and whether it is told. Sent to another PC, it says so: turned on here, it is
 * sent here instead. Held in the background, it says so, and asks again.
 */
@Composable
fun AutoSendCard(pc: Pc, pcs: List<Pc>) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var to by remember { mutableStateOf(AutoSend.to(context)) }
    var screenshots by remember { mutableStateOf(AutoSend.screenshots(context)) }
    var videos by remember { mutableStateOf(AutoSend.videos(context)) }
    var notice by remember { mutableStateOf(AutoSend.notice(context)) }
    var turning by remember { mutableStateOf(false) }
    // Why it could not be done, said over the page.
    var trouble by remember { mutableStateOf<String?>(null) }
    val on = to == pc.key

    fun granted(permission: String) = context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
    val modern = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
    // (Android 13 on asks for photos and videos apart; before, for the storage.)
    val photos = AutoSend.permission('i')
    val movies = AutoSend.permission('v')
    // Whether the photos may be read: taken away in the system's settings, nothing is sent till they may again.
    var readable by remember { mutableStateOf(granted(photos)) }
    // Whether the system lets the app be in the background: held there, nothing is sent till it is opened.
    var unheld by remember { mutableStateOf(AutoSend.unheld(context)) }
    val lettingGo = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { unheld = AutoSend.unheld(context) }
    fun letGo() = lettingGo.launch(AutoSend.letGo(context))
    // Whether photos were found only long after they were taken, where the system said it lets the app be.
    var held by remember { mutableStateOf(AutoSend.held(context)) }
    // Back from elsewhere (the system's settings, perhaps), it is read again as it is now.
    LifecycleResumeEffect(pc.key) {
        to = AutoSend.to(context)
        val was = readable
        readable = granted(photos)
        unheld = AutoSend.unheld(context)
        held = AutoSend.held(context)
        if (readable && !was && to != null) kotlin.concurrent.thread(name = "auto-send-resume") { AutoSend.resume(context.applicationContext) }
        // What was taken meanwhile is looked at: found long after it was taken, the system held the app.
        if (to != null && readable) scope.launch {
            withContext(Dispatchers.IO) { AutoSend.look(context.applicationContext) }
            held = AutoSend.held(context)
        }
        onPauseOrDispose {}
    }
    // Android 14 on lets its user give some photos alone: then new ones are not seen.
    val some = Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE

    var after by remember { mutableStateOf<() -> Unit>({}) }
    val asking = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { after() }
    fun ask(permissions: List<String>, then: () -> Unit) {
        val missing = permissions.filterNot(::granted).distinct()
        if (missing.isEmpty()) return then()
        after = then
        asking.launch(missing.toTypedArray())
    }

    fun turnOn() {
        val wanted = listOf(photos) + listOfNotNull(
            Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED.takeIf { some },
            movies.takeIf { videos },
            Manifest.permission.POST_NOTIFICATIONS.takeIf { modern && notice },
        )
        ask(wanted) {
            when {
                granted(photos) -> {}
                some && granted(Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED) -> return@ask run { trouble = context.getString(R.string.auto_partial) }
                else -> return@ask run { trouble = context.getString(R.string.auto_denied) }
            }
            readable = true
            if (videos && !granted(movies)) {
                AutoSend.setVideos(context, false)
                videos = false
            }
            turning = true
            scope.launch {
                val failed = withContext(Dispatchers.IO) {
                    try {
                        AutoSend.turnOn(context, pc)
                        null
                    } catch (off: AutoSend.Off) {
                        context.getString(R.string.auto_pc_off)
                    } catch (away: Failure.Unreachable) {
                        context.getString(R.string.auto_unreachable)
                    } catch (refused: Failure.Refused) {
                        context.getString(R.string.auto_refused)
                    } catch (failed: Failure.Failed) {
                        context.getString(R.string.auto_said, failed.why)
                    }
                }
                turning = false
                to = AutoSend.to(context)
                held = AutoSend.held(context)
                trouble = failed
                if (failed == null && !AutoSend.unheld(context)) letGo()
            }
        }
    }

    trouble?.let { Tell(stringResource(R.string.auto_failed), it, stringResource(R.string.close)) { trouble = null } }
    Text(pc.name, color = Dim, fontSize = 13.sp, fontWeight = FontWeight.Medium, modifier = Modifier.padding(start = 16.dp, top = 8.dp))
    Panel {
        Column(Modifier.animateContentSize()) {
            val elsewhere = pcs.firstOrNull { it.key == to && it.key != pc.key }
            val what = when {
                elsewhere != null -> stringResource(R.string.auto_send_elsewhere, elsewhere.name)
                on && !readable -> stringResource(R.string.auto_no_access)
                else -> stringResource(R.string.auto_send_what)
            }
            Line(stringResource(R.string.auto_send), what) {
                Toggle(on, stringResource(R.string.auto_send), enabled = !turning) { wanted ->
                    if (wanted) {
                        turnOn()
                    } else {
                        AutoSend.turnOff(context)
                        to = null
                    }
                }
            }
            if (on) {
                // Held in the background: the system's question where it was not let be, else its settings for the app.
                if (readable && (!unheld || held)) {
                    Rule()
                    Line(stringResource(R.string.auto_held), stringResource(if (unheld) R.string.auto_held_late else R.string.auto_held_what)) {
                        if (unheld) {
                            Words(stringResource(R.string.auto_held_settings), Accent) { context.startActivity(AutoSend.settings(context)) }
                        } else {
                            Words(stringResource(R.string.auto_held_allow), Accent, press = ::letGo)
                        }
                    }
                }
                Rule()
                Line(stringResource(R.string.auto_screenshots), null) {
                    Toggle(screenshots, stringResource(R.string.auto_screenshots)) { AutoSend.setScreenshots(context, it); screenshots = it }
                }
                Rule()
                Line(stringResource(R.string.auto_videos), null) {
                    Toggle(videos, stringResource(R.string.auto_videos)) { wanted ->
                        if (!wanted) {
                            AutoSend.setVideos(context, false)
                            videos = false
                        } else {
                            ask(listOf(movies)) {
                                if (granted(movies)) {
                                    AutoSend.setVideos(context, true)
                                    videos = true
                                } else {
                                    trouble = context.getString(R.string.auto_videos_denied)
                                }
                            }
                        }
                    }
                }
                Rule()
                Line(stringResource(R.string.auto_notice), null) {
                    Toggle(notice, stringResource(R.string.auto_notice)) { wanted ->
                        AutoSend.setNotice(context, wanted)
                        notice = wanted
                        if (wanted && modern) ask(listOf(Manifest.permission.POST_NOTIFICATIONS)) {}
                    }
                }
            }
        }
    }
}
