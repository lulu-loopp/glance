package dev.weiyi.glance.remote

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog

// The app's one look: dark and flat. Everything sits on one grid: 16 from
// the screen's sides, 16 inside a card, 12 between cards, 8 between small
// things; nothing pressed is smaller than 44.
val Ground = Color(0xFF0E1116)
val Card = Color(0xFF171B22)
val Card2 = Color(0xFF20252D)
val Hair = Color(0xFF2B313B)
val Ink = Color(0xFFF2F4F7)
val Dim = Color(0xFF98A2AE)
val Faint = Color(0xFF5B6470)
val Accent = Color(0xFFFF7B4A)
val Lit = Color(0xFF2A1D17)
val Good = Color(0xFF58C98B)
val Warm = Color(0xFFFFB057)
val Hot = Color(0xFFFF6B5E)
val Bad = Color(0xFFF0685D)

/** The app's drawn signs, each a path on a 24 by 24 square, stroked. */
object Sign {
    const val NIGHT = "M20.4 14.6A8.6 8.6 0 0 1 9.4 3.6a8.6 8.6 0 1 0 11 11z"
    const val SCREEN = "M3.5 5h17v11.5h-17zM8.5 20h7M12 16.5V20"
    const val LIGHTS = "M9.2 18h5.6M10.3 21h3.4M12 3a6.2 6.2 0 0 0-4 11c.8.7 1.2 1.6 1.2 2.6h5.6c0-1 .4-1.9 1.2-2.6A6.2 6.2 0 0 0 12 3z"
    const val SLEEP = "M12 3v8M6.6 6.6a7.6 7.6 0 1 0 10.8 0"
    const val GEAR = "M12 8.6a3.4 3.4 0 1 0 0 6.8 3.4 3.4 0 0 0 0-6.8zM19.2 12a7.2 7.2 0 0 0-.1-1.2l2-1.5-2-3.4-2.3.9a7.3 7.3 0 0 0-2.1-1.2L14.3 3h-4.6l-.4 2.6a7.3 7.3 0 0 0-2.1 1.2l-2.3-.9-2 3.4 2 1.5a7.2 7.2 0 0 0 0 2.4l-2 1.5 2 3.4 2.3-.9a7.3 7.3 0 0 0 2.1 1.2l.4 2.6h4.6l.4-2.6a7.3 7.3 0 0 0 2.1-1.2l2.3.9 2-3.4-2-1.5c.1-.4.1-.8.1-1.2z"
    const val BACK = "M15 5l-7 7 7 7"
    const val PLAY = "M8 5.5v13l11-6.5z"
    const val PAUSE = "M8 5v14M16 5v14"
    const val PREVIOUS = "M18 6v12l-9-6zM6.5 6v12"
    const val NEXT = "M6 6v12l9-6zM17.5 6v12"
    const val BRIGHT = "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"
    // What is handed to a PC (Lucide's upload, image, file, link, pilcrow, monitor, check, circle-alert).
    const val UPLOAD = "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12"
    const val IMAGE = "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM9 7a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM21 15l-3.1-3.1a2 2 0 0 0-2.8 0L6 21"
    const val FILE = "M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7zM14 2v4a2 2 0 0 0 2 2h4"
    const val LINK = "M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7.1-7.1l-1.7 1.7M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7.1 7.1l1.7-1.7"
    const val TEXT = "M13 4v16M17 4v16M19 4H9.5a4.5 4.5 0 0 0 0 9H13"
    const val MONITOR = "M4 3h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM8 21h8M12 17v4"
    const val CHECK = "M20 6L9 17l-5-5"
    const val ALERT = "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zM12 8v4M12 16h.01"
}

@Composable
fun Sign(path: String, tint: Color, modifier: Modifier = Modifier, size: Dp = 22.dp) {
    val drawn = remember(path) { PathParser().parsePathString(path).toPath() }
    Canvas(modifier.size(size)) {
        scale(this.size.width / 24f, pivot = Offset.Zero) { drawPath(drawn, tint, style = Stroke(1.7f, cap = StrokeCap.Round, join = StrokeJoin.Round)) }
    }
}

/** A sign to press, in a round of 44. */
@Composable
fun SignButton(path: String, label: String, modifier: Modifier = Modifier, tint: Color = Dim, ground: Color = Color.Transparent, enabled: Boolean = true, press: () -> Unit) {
    Box(
        modifier.size(44.dp).clip(CircleShape).background(ground).clickable(enabled = enabled, role = Role.Button, onClick = press).semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) { Sign(path, if (enabled) tint else Faint) }
}

/** A card: what belongs together, on one ground. */
@Composable
fun Panel(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(modifier.fillMaxWidth().clip(RoundedCornerShape(20.dp)).background(Card).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp), content = content)
}

/** What a card is of, small above it; and, at its right, something of its own. */
@Composable
fun Heading(text: String, right: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().heightIn(min = 20.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(text, color = Dim, fontSize = 13.sp, fontWeight = FontWeight.Medium, modifier = Modifier.weight(1f))
        right()
    }
}

/** One thing to ask, as a large button: lit while it is on. */
@Composable
fun Tile(sign: String, title: String, detail: String, lit: Boolean, enabled: Boolean, modifier: Modifier = Modifier, big: Boolean = false, press: () -> Unit) {
    Column(
        modifier.heightIn(min = if (big) 80.dp else 92.dp).clip(RoundedCornerShape(20.dp)).background(if (lit && enabled) Lit else Card)
            .clickable(enabled = enabled, role = Role.Button, onClick = press).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
    ) {
        val ink = if (!enabled) Dim else if (lit) Accent else Ink
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Sign(sign, ink, size = 20.dp)
            Text(title, fontSize = if (big) 19.sp else 17.sp, fontWeight = FontWeight.SemiBold, color = ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Text(detail, color = Dim, fontSize = 12.5.sp, lineHeight = 18.sp)
    }
}

/** A word to choose among others, in a pill. */
@Composable
fun Chip(text: String, chosen: Boolean, ground: Color = Card2, lit: Color = Ink, press: () -> Unit) {
    Box(Modifier.height(36.dp).clip(RoundedCornerShape(50)).background(if (chosen) lit else ground).clickable(role = Role.Tab, onClick = press).padding(horizontal = 14.dp), contentAlignment = Alignment.Center) {
        Text(text, fontSize = 13.5.sp, fontWeight = if (chosen) FontWeight.SemiBold else FontWeight.Normal, color = if (chosen) Color.Black else Dim, maxLines = 1)
    }
}

/** A switch: its track takes the accent when on; its thumb is white either way. */
@Composable
fun Toggle(on: Boolean, label: String, enabled: Boolean = true, change: (Boolean) -> Unit) {
    val at by animateFloatAsState(if (on) 1f else 0f, tween(160), label = "toggle")
    Box(Modifier.heightIn(min = 44.dp).toggleable(on, enabled = enabled, role = Role.Switch, onValueChange = change).semantics { contentDescription = label }, contentAlignment = Alignment.Center) {
        Canvas(Modifier.size(50.dp, 30.dp).alpha(if (enabled) 1f else 0.4f)) {
            drawRoundRect(lerp(Color(0xFF353C47), Accent, at), cornerRadius = CornerRadius(size.height / 2))
            drawCircle(Color.White, 12.dp.toPx(), Offset(15.dp.toPx() + 20.dp.toPx() * at, size.height / 2))
        }
    }
}

/**
 * A bar slid along: [value] of [range], on a track painted by [track]
 * (given the fraction along). [change] as it is slid, [done] when let go.
 * [slim], it is a thin line with a small thumb: for what is set beside
 * something else, not by itself.
 */
@Composable
fun Bar(value: Float, range: ClosedFloatingPointRange<Float>, label: String, modifier: Modifier = Modifier, enabled: Boolean = true, slim: Boolean = false, track: (Float) -> Brush, change: (Float) -> Unit, done: () -> Unit) {
    val changed by rememberUpdatedState(change)
    val finished by rememberUpdatedState(done)
    val span = range.endInclusive - range.start
    val along = ((value - range.start) / span).coerceIn(0f, 1f)
    Canvas(
        modifier.fillMaxWidth().height(28.dp).alpha(if (enabled) 1f else 0.4f).semantics { contentDescription = label }
            .pointerInput(enabled, range, slim) {
                if (!enabled) return@pointerInput
                val edge = (if (slim) 9.dp else 13.dp).toPx()
                fun at(x: Float) = range.start + ((x - edge) / (size.width - 2 * edge)).coerceIn(0f, 1f) * span
                detectTapGestures { changed(at(it.x)); finished() }
            }
            .pointerInput(enabled, range, slim) {
                if (!enabled) return@pointerInput
                val edge = (if (slim) 9.dp else 13.dp).toPx()
                fun at(x: Float) = range.start + ((x - edge) / (size.width - 2 * edge)).coerceIn(0f, 1f) * span
                detectHorizontalDragGestures(onDragStart = { changed(at(it.x)) }, onDragEnd = { finished() }, onDragCancel = { finished() }) { touch, _ ->
                    touch.consume()
                    changed(at(touch.position.x))
                }
            },
    ) {
        val thick = (if (slim) 6.dp else 16.dp).toPx()
        val edge = (if (slim) 9.dp else 13.dp).toPx()
        drawRoundRect(track(along), Offset(0f, (size.height - thick) / 2), Size(size.width, thick), CornerRadius(thick / 2))
        val centre = Offset(edge + (size.width - 2 * edge) * along, size.height / 2)
        drawCircle(Color(0x66000000), edge + 1.dp.toPx(), centre + Offset(0f, 1.dp.toPx()))
        drawCircle(Color.White, edge, centre)
    }
}

/** A track filled up to where its bar is. */
fun filled(color: Color): (Float) -> Brush = { along -> Brush.horizontalGradient(0f to color, along to color, along to Card2, 1f to Card2) }

/** A row of a list: a name and a word under it, then what is pressed, at the card's right edge. */
@Composable
fun Line(title: String, detail: String?, modifier: Modifier = Modifier, lead: @Composable () -> Unit = {}, note: String? = null, right: @Composable () -> Unit) {
    Row(modifier.fillMaxWidth().heightIn(min = 56.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        lead()
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(title, fontSize = 15.sp, fontWeight = FontWeight.Medium, color = Ink, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                if (note != null) Text(note, fontSize = 12.sp, color = Accent, maxLines = 1)
            }
            if (!detail.isNullOrEmpty()) Text(detail, fontSize = 12.5.sp, color = Dim, lineHeight = 17.sp)
        }
        right()
    }
}

/** The line between two rows of a list. */
@Composable
fun Rule() = Box(Modifier.fillMaxWidth().height(1.dp).background(Hair))

/** Words to press, with no box of their own. */
@Composable
fun Words(text: String, color: Color, modifier: Modifier = Modifier, size: Float = 14f, weight: FontWeight = FontWeight.Normal, enabled: Boolean = true, press: () -> Unit) {
    Box(modifier.heightIn(min = 44.dp).clip(RoundedCornerShape(12.dp)).clickable(enabled = enabled, role = Role.Button, onClick = press), contentAlignment = Alignment.Center) {
        Text(text, color = if (enabled) color else Faint, fontSize = size.sp, fontWeight = weight, maxLines = 1)
    }
}

/** Something the app has to say (why it could not do what was asked), and a word to put it away. */
@Composable
fun Tell(title: String, text: String, close: String, dismiss: () -> Unit) {
    Dialog(onDismissRequest = dismiss) {
        Column(Modifier.widthIn(max = 328.dp).clip(RoundedCornerShape(24.dp)).background(Card2).padding(start = 24.dp, top = 24.dp, end = 12.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(title, fontSize = 19.sp, fontWeight = FontWeight.SemiBold, color = Ink, modifier = Modifier.padding(end = 12.dp))
            Text(text, fontSize = 14.sp, lineHeight = 22.sp, color = Dim, modifier = Modifier.padding(end = 12.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                Words(close, Ink, Modifier.padding(horizontal = 4.dp).widthIn(min = 56.dp), size = 15f, weight = FontWeight.SemiBold, press = dismiss)
            }
        }
    }
}

/** A question over the app: what it is, what doing it means, and the two answers. */
/** Something long to read, in a box that scrolls, and a word to put it away. */
@Composable
fun Read(title: String, text: String, close: String, dismiss: () -> Unit) {
    Dialog(onDismissRequest = dismiss) {
        Column(Modifier.widthIn(max = 420.dp).clip(RoundedCornerShape(24.dp)).background(Card2).padding(start = 24.dp, top = 24.dp, end = 12.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(title, fontSize = 19.sp, fontWeight = FontWeight.SemiBold, color = Ink, modifier = Modifier.padding(end = 12.dp))
            Text(text, fontSize = 12.sp, lineHeight = 18.sp, color = Dim, modifier = Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()).padding(end = 12.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                Words(close, Ink, Modifier.padding(horizontal = 4.dp).widthIn(min = 56.dp), size = 15f, weight = FontWeight.SemiBold, press = dismiss)
            }
        }
    }
}

@Composable
fun Ask(title: String, paragraphs: List<String>, verb: String, cancel: String, dismiss: () -> Unit, done: () -> Unit) {
    Dialog(onDismissRequest = dismiss) {
        Column(Modifier.widthIn(max = 328.dp).clip(RoundedCornerShape(24.dp)).background(Card2).padding(start = 24.dp, top = 24.dp, end = 12.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(title, fontSize = 19.sp, fontWeight = FontWeight.SemiBold, color = Ink, modifier = Modifier.padding(end = 12.dp))
            for (text in paragraphs) Text(text, fontSize = 14.sp, lineHeight = 22.sp, color = Dim, modifier = Modifier.padding(end = 12.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                Words(cancel, Dim, Modifier.padding(horizontal = 4.dp).widthIn(min = 56.dp), size = 15f, weight = FontWeight.SemiBold, press = dismiss)
                Words(verb, Bad, Modifier.padding(horizontal = 4.dp).widthIn(min = 56.dp), size = 15f, weight = FontWeight.SemiBold) { dismiss(); done() }
            }
        }
    }
}
