package dev.weiyi.glance.remote

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.system.OsConstants
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.material3.Text
import androidx.compose.ui.window.Dialog
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException
import java.net.ConnectException
import java.net.HttpURLConnection
import java.net.Inet4Address
import java.net.Inet6Address
import java.net.InetAddress
import java.net.NoRouteToHostException
import java.net.SocketTimeoutException
import java.net.URL
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLException
import javax.net.ssl.X509TrustManager
import kotlin.concurrent.thread

/**
 * Where devices reach this phone, and devices added by their addresses: on a campus network devices on other networks
 * are not found nearby (what announces them is not passed between networks), but are reached by their addresses, an
 * IPv6 one above all (see `docs/nearby.md`, "By address"). A device added is known by the certificate it proved
 * holding, kept with the addresses it answered at and tells of (the one that last answered first), and told of this
 * phone now and then while it receives, so that each finds the other.
 */
object Address {
    /** Glance's PCs receive on this port (LocalSend's own, [Lan.PORT], is looked on too). */
    const val GLANCE_PORT = 53320
    /** The most addresses a device added is kept with. */
    private const val MOST = 8

    /** A device added by its address: as it says it is, how it receives, and where (each "host:port", the one that last answered first). */
    data class Added(val fingerprint: String, val alias: String, val model: String?, val type: String?, val glance: Boolean, val https: Boolean, val addresses: List<String>)

    /** How adding a device went: added, listed already, or why not. */
    enum class Outcome { Added, Listed, NotAnAddress, Unreachable, Itself }

    // ---- Where this phone is reached ----

    /** One of this phone's addresses: on which network ("WLAN", "以太网"), whether it is IPv6, and the address. */
    class Mine(val network: String, val v6: Boolean, val address: String)

    /**
     * This phone's addresses on the networks other devices reach it on (Wi-Fi and Ethernet: a mobile network lets
     * nothing in): on each, its IPv6 address meant to last first (one of the Internet's, not one made up for privacy,
     * replaced within a day), then its IPv4 ones.
     */
    fun mine(context: Context): List<Mine> {
        val networks = context.getSystemService(ConnectivityManager::class.java) ?: return emptyList()
        @Suppress("DEPRECATION")
        return networks.allNetworks.flatMap { network ->
            val can = networks.getNetworkCapabilities(network) ?: return@flatMap emptyList()
            val name = when {
                can.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> context.getString(R.string.net_wlan)
                can.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> context.getString(R.string.net_ethernet)
                else -> return@flatMap emptyList()
            }
            val links = networks.getLinkProperties(network)?.linkAddresses.orEmpty()
            val v6 = links.firstOrNull { link ->
                val address = link.address
                address is Inet6Address && global(address) && link.flags and (OsConstants.IFA_F_TEMPORARY or OsConstants.IFA_F_DEPRECATED) == 0
            }?.address?.let { Mine(name, true, plain(it)) }
            val v4 = links.map { it.address }.filterIsInstance<Inet4Address>().filter { !it.isLoopbackAddress && !it.isLinkLocalAddress }.map { Mine(name, false, plain(it)) }
            listOfNotNull(v6) + v4
        }.distinctBy { it.address }
    }

    /** Whether [address] is one of the Internet's (2000::/3), not a link's own or a network's own. */
    private fun global(address: Inet6Address) = address.address[0].toInt() and 0xE0 == 0x20

    /** [address] as it is written, an IPv6 one without the network it is scoped to. */
    private fun plain(address: InetAddress): String = address.hostAddress.orEmpty().substringBefore('%')

    /** [address] at [port], as devices tell it ("10.0.0.2:53317", "[2001:da8::1]:53317"). */
    fun at(address: String, port: Int) = if (address.contains(':')) "[$address]:$port" else "$address:$port"

    /** "host:port" split, the host without brackets; none where it is not that. */
    fun split(at: String): Pair<String, Int>? {
        val (host, port) = if (at.startsWith('[')) at.substringAfter('[').substringBefore(']') to at.substringAfter("]:", "") else at.substringBeforeLast(':') to at.substringAfterLast(':', "")
        return port.toIntOrNull()?.takeIf { it in 1..65535 && host.isNotEmpty() }?.let { host to it }
    }

    /** "host:port" as it is shown and given: without its port where that is one looked on for an address alone. */
    fun shown(at: String): String = split(at)?.let { (host, port) -> if (port == GLANCE_PORT || port == Lan.PORT) host else at } ?: at

    /** This phone's addresses as devices are told them, each at [port]. */
    fun told(context: Context, port: Int): List<String> = runCatching { mine(context).map { at(it.address, port) } }.getOrDefault(emptyList())

    // ---- Reading what is typed ----

    /**
     * An address as typed (IPv4 or IPv6, with a port or not: "10.21.140.12", "10.21.140.12:53318", "2001:da8::1",
     * "[2001:da8::1]:53318"): the address, and the port given where one is. Nothing but an address is read (no name is
     * looked up).
     */
    fun parse(typed: String): Pair<InetAddress, Int?>? {
        val text = typed.trim()
        if (text.isEmpty()) return null
        val (host, port) = when {
            text.startsWith('[') -> {
                val host = text.substringAfter('[').substringBefore(']', "")
                val rest = text.substringAfter(']', "")
                when {
                    rest.isEmpty() -> host to null
                    rest.startsWith(':') -> host to (rest.drop(1).toIntOrNull()?.takeIf { it in 1..65535 } ?: return null)
                    else -> return null
                }
            }
            text.count { it == ':' } == 1 -> text.substringBefore(':') to (text.substringAfter(':').toIntOrNull()?.takeIf { it in 1..65535 } ?: return null)
            else -> text to null
        }
        val address = if (host.contains(':')) v6(host) else v4(host)
        return address?.let { it to port }
    }

    /** A dotted IPv4 address, four numbers each 0 to 255. */
    private fun v4(text: String): InetAddress? {
        val parts = text.split('.')
        if (parts.size != 4 || parts.any { it.isEmpty() || it.length > 3 || !it.all(Char::isDigit) || it.toInt() > 255 }) return null
        return InetAddress.getByAddress(parts.map { it.toInt().toByte() }.toByteArray())
    }

    /** An IPv6 address (its characters those of one alone, so that nothing is looked up), an IPv4 one it maps read as that. */
    private fun v6(text: String): InetAddress? {
        if (!text.all { it.isDigit() || it in 'a'..'f' || it in 'A'..'F' || it == ':' || it == '.' } || text.count { it == ':' } < 2) return null
        return runCatching { InetAddress.getByName(text) }.getOrNull()
    }

    // ---- Reaching a device by its address ----

    /** How a device was looked for at an address: what it says of itself (known by the certificate it proved), something there that is no LocalSend device, or nothing. */
    private sealed interface Reached {
        class Device(val info: Lan.Info) : Reached
        data object Other : Reached
        data object Nothing : Reached
    }

    /** Takes the certificate the device presents, whichever it is, keeping its fingerprint (the handshake proves the device holds it). */
    private class Seen : X509TrustManager {
        @Volatile
        var fingerprint: String? = null

        override fun checkServerTrusted(chain: Array<out X509Certificate>, authType: String?) {
            if (chain.isEmpty()) throw CertificateException("no certificate")
            fingerprint = Own.fingerprint(chain[0])
        }

        override fun checkClientTrusted(chain: Array<out X509Certificate>, authType: String?) = throw CertificateException("this does not receive")
        override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
    }

    /** What the device at [host]:[port] says of itself, over HTTPS (its fingerprint the one of the certificate it presented), else, where it speaks no TLS there, over plain HTTP. */
    private fun reach(host: String, port: Int): Reached {
        val seen = Seen()
        fun ask(https: Boolean): Lan.Info? {
            val connection = URL("${if (https) "https" else "http"}://${bracketed(host)}:$port${Lan.API}/info").openConnection() as HttpURLConnection
            if (connection is HttpsURLConnection) {
                connection.sslSocketFactory = SSLContext.getInstance("TLS").apply { init(arrayOf(Own.Presenting()), arrayOf(seen), null) }.socketFactory
                connection.setHostnameVerifier { _, _ -> true }
            }
            connection.connectTimeout = 3000
            connection.readTimeout = 5000
            connection.setRequestProperty("Connection", "close")
            return try {
                if (connection.responseCode != 200) null else Lan.Info.of(String(connection.inputStream.use { it.readUpTo(Lan.MOST_INFO) }))
            } finally {
                connection.disconnect()
            }
        }
        return try {
            val info = ask(true) ?: return Reached.Other
            val proven = seen.fingerprint ?: return Reached.Other
            if (!info.fingerprint.equals(proven, ignoreCase = true)) return Reached.Other
            Reached.Device(info.copy(fingerprint = proven.uppercase(), port = port, https = true))
        } catch (plain: SSLException) {
            try {
                ask(false)?.let { Reached.Device(it.copy(fingerprint = it.fingerprint.uppercase(), port = port, https = false)) } ?: Reached.Other
            } catch (failed: IOException) {
                Reached.Other
            }
        } catch (away: ConnectException) {
            Reached.Nothing
        } catch (away: SocketTimeoutException) {
            Reached.Nothing
        } catch (away: NoRouteToHostException) {
            Reached.Nothing
        } catch (failed: IOException) {
            Reached.Other
        }
    }

    private fun bracketed(host: String) = if (host.contains(':')) "[$host]" else host

    /**
     * The device at [typed] (see [parse]) added: reached by its address alone (on the port given, else Glance's PCs'
     * then LocalSend's), told of this phone (it finds this phone then), and kept, where it was not known already (found
     * nearby, a paired PC, or added before). Blocks: called off the main thread.
     */
    fun add(context: Context, typed: String): Outcome {
        val (address, port) = parse(typed) ?: return Outcome.NotAnAddress
        val host = plain(address)
        var reached: Reached = Reached.Nothing
        for (tried in port?.let { listOf(it) } ?: listOf(GLANCE_PORT, Lan.PORT)) {
            reached = reach(host, tried)
            // (Something answered there: what it is, it is, on any other port too.)
            if (reached != Reached.Nothing) break
        }
        val info = (reached as? Reached.Device)?.info ?: return Outcome.Unreachable
        if (info.fingerprint.equals(Own.fingerprint(), ignoreCase = true)) return Outcome.Itself
        val paired = Pcs.fingerprints(context).values.any { it.equals(info.fingerprint, ignoreCase = true) }
        val known = paired || Receiver.devices.value.any { it.fingerprint.equals(info.fingerprint, ignoreCase = true) } || all(context).any { it.fingerprint.equals(info.fingerprint, ignoreCase = true) }
        if (!known) {
            val told = info.addresses.filter { split(it) != null }
            keep(context, all(context) + Added(info.fingerprint, info.alias, info.deviceModel, info.deviceType, info.glance, info.https, (listOf(at(host, info.port ?: Lan.PORT)) + told).distinct().take(MOST)))
        }
        tell(context, info.fingerprint, host, info.port ?: Lan.PORT, info.https)
        return if (known) Outcome.Listed else Outcome.Added
    }

    /** The device of [fingerprint] at [host]:[port] told of this phone, and heard as it answers (by its certificate, over HTTPS). Whether it answered. */
    private fun tell(context: Context, fingerprint: String, host: String, port: Int, https: Boolean): Boolean {
        val answered = Found.registerWith(host, port, https, fingerprint, Receiver.me(context)) ?: return false
        Receiver.heard(answered.copy(port = port, https = https), host, fingerprint.uppercase().takeIf { https })
        return true
    }

    /** Each device added told of this phone (a thread each): at the address that last answered first, then at each other, till one answers. */
    fun reachAll(context: Context) {
        for (added in all(context)) {
            thread(name = "nearby-added", isDaemon = true) {
                for (at in added.addresses) {
                    val (host, port) = split(at) ?: continue
                    if (tell(context, added.fingerprint, host, port, added.https)) break
                }
            }
        }
    }

    // ---- What is kept ----

    private fun kept(context: Context) = context.getSharedPreferences("nearby", Context.MODE_PRIVATE)

    /** The devices added by their addresses, as they are kept. */
    fun all(context: Context): List<Added> {
        val list = runCatching { JSONArray(kept(context).getString("added", "[]")) }.getOrElse { JSONArray() }
        return (0 until list.length()).mapNotNull { at ->
            val one = list.optJSONObject(at) ?: return@mapNotNull null
            val addresses = one.optJSONArray("addresses")?.let { array -> (0 until array.length()).map { array.optString(it) }.filter { split(it) != null } }.orEmpty()
            Added(one.optString("fingerprint"), one.optString("alias"), one.optString("model").ifEmpty { null }, one.optString("type").ifEmpty { null }, one.optBoolean("glance"), one.optBoolean("https", true), addresses)
                .takeIf { it.fingerprint.isNotEmpty() }
        }
    }

    @Synchronized
    private fun keep(context: Context, list: List<Added>) {
        val json = JSONArray(list.map { JSONObject().put("fingerprint", it.fingerprint).put("alias", it.alias).put("model", it.model.orEmpty()).put("type", it.type.orEmpty()).put("glance", it.glance).put("https", it.https).put("addresses", JSONArray(it.addresses)) })
        kept(context).edit().putString("added", json.toString()).apply()
        Kept.change()
    }

    /** The device added of [fingerprint] no longer kept. */
    @Synchronized
    fun remove(context: Context, fingerprint: String) = keep(context, all(context).filterNot { it.fingerprint.equals(fingerprint, ignoreCase = true) })

    /**
     * A device added that proved itself heard at [host]:[port], telling of [others]: kept as reached there first, then
     * at those, then where it was known before, its name as it says now. Nothing for a device not added.
     */
    @Synchronized
    fun reached(context: Context, fingerprint: String, host: String, port: Int, info: Lan.Info) {
        val list = all(context)
        val was = list.firstOrNull { it.fingerprint.equals(fingerprint, ignoreCase = true) } ?: return
        val addresses = (listOf(at(host, port)) + info.addresses.filter { split(it) != null } + was.addresses).distinct().take(MOST)
        val now = was.copy(alias = info.alias, model = info.deviceModel ?: was.model, addresses = addresses)
        if (now != was) keep(context, list.map { if (it === was) now else it })
    }
}

/** How long "已复制" is said after an address is copied (ms). */
private const val COPIED_FOR = 2000L

/**
 * This phone's addresses, in the nearby card: what they are for, then each on its own row (the address, its network
 * and kind under it), with what copies it.
 */
@Composable
fun MyAddresses() {
    val context = LocalContext.current
    val mine = remember { Address.mine(context) }
    var copied by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(copied) {
        if (copied != null) {
            delay(COPIED_FOR)
            copied = null
        }
    }
    Line(stringResource(R.string.near_addresses), stringResource(if (mine.isEmpty()) R.string.near_addresses_none else R.string.near_addresses_what)) {}
    for (address in mine) {
        Rule()
        Line(address.address, "${address.network} · ${if (address.v6) "IPv6" else "IPv4"}") {
            Words(stringResource(if (copied == address.address) R.string.copied else R.string.copy), Accent) {
                context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText("IP", address.address))
                copied = address.address
            }
        }
    }
}

/**
 * The address of a device to add written in a box: tried as it is added ("正在连接", the box and 添加 greyed), then
 * gone where it was added, else what went wrong said under the box.
 */
@Composable
fun AddByAddress(dismiss: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var typed by remember { mutableStateOf("") }
    var trying by remember { mutableStateOf(false) }
    var said by remember { mutableStateOf<String?>(null) }
    val focus = remember { FocusRequester() }
    val add = add@{
        if (trying || typed.isBlank()) return@add
        trying = true
        said = context.getString(R.string.address_trying)
        scope.launch {
            val written = typed.trim()
            val outcome = withContext(Dispatchers.IO) { Address.add(context, written) }
            trying = false
            said = when (outcome) {
                Address.Outcome.Added -> return@launch dismiss()
                Address.Outcome.Listed -> context.getString(R.string.address_listed)
                Address.Outcome.NotAnAddress -> context.getString(R.string.address_bad)
                Address.Outcome.Itself -> context.getString(R.string.address_itself)
                Address.Outcome.Unreachable -> context.getString(R.string.address_unreachable, written) + "\n" + context.getString(R.string.address_unreachable_what)
            }
        }
    }
    Dialog(onDismissRequest = dismiss) {
        Column(Modifier.widthIn(max = 328.dp).clip(RoundedCornerShape(24.dp)).background(Card2).padding(start = 24.dp, top = 24.dp, end = 12.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.address_add), fontSize = 19.sp, fontWeight = FontWeight.SemiBold, color = Ink)
            Row(Modifier.padding(end = 12.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Card).padding(horizontal = 12.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                BasicTextField(
                    typed, { typed = it.take(64); said = null },
                    Modifier.fillMaxWidth().focusRequester(focus),
                    enabled = !trying, singleLine = true, textStyle = TextStyle(color = if (trying) Dim else Ink, fontSize = 15.sp), cursorBrush = SolidColor(Accent),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    decorationBox = { field ->
                        if (typed.isEmpty()) Text(stringResource(R.string.address_placeholder), color = Faint, fontSize = 15.sp)
                        field()
                    },
                )
            }
            val bad = said != null && !trying && said != context.getString(R.string.address_listed)
            Text(said ?: stringResource(R.string.address_where), fontSize = 13.sp, color = if (bad) Bad else Dim, modifier = Modifier.padding(end = 12.dp).heightIn(min = 18.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                Words(stringResource(R.string.cancel), Dim, Modifier.padding(horizontal = 4.dp).widthIn(min = 56.dp), size = 15f, weight = FontWeight.SemiBold, press = dismiss)
                Words(stringResource(R.string.address_add_button), Accent, Modifier.padding(horizontal = 4.dp).widthIn(min = 56.dp), size = 15f, weight = FontWeight.SemiBold, enabled = !trying && typed.isNotBlank(), press = add)
            }
        }
    }
    LaunchedEffect(Unit) { focus.requestFocus() }
}
