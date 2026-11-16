package dev.weiyi.glance.remote

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream

class LanTest {
    private val key = "0123456789abcdef".repeat(4)

    // ---- Pairing links ----

    @Test
    fun pairsByTheRemotePageLink() {
        val pc = Lan.pairing("http://192.168.1.20:47321/#k=$key&n=Weiyi%E7%9A%84%E7%94%B5%E8%84%91")
        assertEquals(Pc("192.168.1.20", 47321, key, "Weiyi的电脑"), pc)
    }

    @Test
    fun pairsByTheOlderGlanceLink() {
        val pc = Lan.pairing("glance://pair?h=10.0.0.19&p=47321&k=${key.uppercase()}&n=Desk+PC")
        assertEquals(Pc("10.0.0.19", 47321, key, "Desk PC"), pc)
    }

    @Test
    fun pairsWithAnAddressOfIpv6() {
        val pc = Lan.pairing("http://[fe80::1]:8080/#k=$key&n=PC")
        assertEquals("fe80::1", pc?.host)
        assertEquals(8080, pc?.port)
        assertEquals("fd00::5", Lan.pairing("http://[fd00::5]:80/#k=$key")?.host)
    }

    @Test
    fun aLinkWithNoNameIsNamedByItsAddress() {
        assertEquals("10.0.0.2", Lan.pairing("http://10.0.0.2:47321/#k=$key")?.name)
        // (A name of control characters only is none.)
        assertEquals("10.0.0.2", Lan.pairing("http://10.0.0.2:47321/#k=$key&n=%01%02")?.name)
    }

    @Test
    fun aNameIsShownOnOneLineAndNotTooLong() {
        val pc = Lan.pairing("http://10.0.0.2:47321/#k=$key&n=" + "a%0Ab".repeat(40))
        assertEquals(64, pc?.name?.length)
        assertFalse(pc!!.name.contains('\n'))
    }

    @Test
    fun linksThatAreNoPairingAreRefused() {
        val refused = listOf(
            "",
            "hello",
            "http://10.0.0.2:47321/",
            "http://10.0.0.2:47321/?k=$key",
            "http://10.0.0.2/#k=$key",
            "http://10.0.0.2:0/#k=$key",
            "http://10.0.0.2:65536/#k=$key",
            "http://10.0.0.2:47321/#k=${key.dropLast(1)}",
            "http://10.0.0.2:47321/#k=${key.dropLast(1)}g",
            "http://user@10.0.0.2:47321/#k=$key",
            "ftp://10.0.0.2:47321/#k=$key",
            "glance://pair?h=&p=47321&k=$key",
            "glance://pair?h=10.0.0.2&p=port&k=$key",
            "glance://pair?h=10.0.0.2&p=47321",
        )
        for (link in refused) assertNull(link, Lan.pairing(link))
    }

    @Test
    fun escapingAndUnescapingGoBack() {
        val words = "a b/c?d=e&f%g+h·文件"
        assertEquals(words, Lan.unescaped(Lan.escaped(words)))
        assertEquals("a b", Lan.unescaped("a+b"))
        // (A "%" not followed by two hex digits is itself.)
        assertEquals("100%", Lan.unescaped("100%"))
        assertEquals("%zz", Lan.unescaped("%zz"))
        assertEquals("%4", Lan.unescaped("%4"))
    }

    // ---- Where what comes is kept ----

    private val types = mapOf("jpg" to "image/jpeg", "png" to "image/png", "mp4" to "video/mp4", "pdf" to "application/pdf", "txt" to "text/plain")

    private fun place(sent: String) = Lan.place(sent) { types[it] }

    @Test
    fun filesGoWhereTheirKindGoes() {
        assertEquals(Lan.Shelf.Pictures, place("IMG_0001.JPG").shelf)
        assertEquals("Pictures/Glance/", place("IMG_0001.JPG").relative)
        assertEquals("image/jpeg", place("IMG_0001.JPG").type)
        assertEquals(Lan.Shelf.Movies, place("clip.mp4").shelf)
        assertEquals("Movies/Glance/", place("clip.mp4").relative)
        assertEquals(Lan.Shelf.Download, place("paper.pdf").shelf)
        assertEquals(Lan.Shelf.Download, place("unknown.xyz").shelf)
        assertEquals("application/octet-stream", place("unknown.xyz").type)
        assertEquals("application/octet-stream", place("noextension").type)
    }

    @Test
    fun aFolderKeepsItsInnerPlacesUnderDownloads() {
        val kept = place("Trip/Day 1/IMG_0001.jpg")
        assertEquals(Lan.Shelf.Download, kept.shelf)
        assertEquals(listOf("Trip", "Day 1"), kept.folders)
        assertEquals("IMG_0001.jpg", kept.name)
        assertEquals("Download/Glance/Trip/Day 1/", kept.relative)
        assertEquals(listOf("Trip", "inner"), place("Trip\\inner\\a.txt").folders)
    }

    @Test
    fun nothingReachesOutOfWhereItIsPut() {
        assertEquals(listOf("etc", "passwd"), Lan.parts("../../etc/passwd"))
        assertEquals(listOf("etc", "passwd"), Lan.parts("/etc/passwd"))
        assertEquals(listOf("a", "b"), Lan.parts("a/./../b"))
        assertEquals(listOf("a", "b"), Lan.parts("a\\..\\..\\b"))
        assertEquals(listOf("file"), Lan.parts("../.."))
        assertEquals(listOf("file"), Lan.parts(""))
        assertEquals(listOf("file"), Lan.parts("/ / . "))
        // (".." made of more dots, or with spaces, is no way up once cleaned either.)
        assertEquals(listOf("file"), Lan.parts(". ./.. /..."))
        for (sent in listOf("../x", "a/../../x", "..\\..\\x", "C:\\Windows\\x", "a/%2e%2e/x")) {
            for (part in place(sent).folders + place(sent).name) {
                assertFalse(sent, part == ".." || part == "." || part.isEmpty() || part.contains('/') || part.contains('\\'))
            }
        }
    }

    @Test
    fun aPartIsMadeAPlainName() {
        assertEquals("a_b_c_d_e_f_g", Lan.cleanPart("a:b*c?d\"e<f>g"))
        assertEquals("a_b", Lan.cleanPart("a|b"))
        assertEquals("tab here", Lan.cleanPart("tab\there"))
        assertEquals("name", Lan.cleanPart("  name. . "))
        assertEquals("C_", Lan.cleanPart("C:"))
    }

    @Test
    fun aLongNameIsCutKeepingItsExtension() {
        val long = Lan.cleanPart("字".repeat(300) + ".jpeg")
        assertTrue(long.toByteArray().size <= 200)
        assertTrue(long.endsWith(".jpeg"))
        assertTrue(long.startsWith("字"))
        val plain = Lan.cleanPart("a".repeat(500))
        assertEquals(200, plain.length)
        // (An "extension" too long to be one is cut with the rest.)
        val odd = Lan.cleanPart("a".repeat(300) + "." + "b".repeat(30))
        assertEquals(200, odd.length)
    }

    // ---- Who is taken from ----

    private val paired = setOf("AA".repeat(32))
    private val trusted = setOf("bb".repeat(32))

    @Test
    fun pairedAndTrustedAreTakenByTheirCertificate() {
        assertEquals(Lan.Decision.Take, Lan.decide("aa".repeat(32), paired, trusted, Lan.Others.Ask))
        assertEquals(Lan.Decision.Take, Lan.decide("BB".repeat(32), paired, trusted, Lan.Others.Refuse))
    }

    @Test
    fun othersAreAskedAboutOrRefused() {
        assertEquals(Lan.Decision.Ask(canTrust = true), Lan.decide("CC".repeat(32), paired, trusted, Lan.Others.Ask))
        assertEquals(Lan.Decision.Refuse, Lan.decide("CC".repeat(32), paired, trusted, Lan.Others.Refuse))
    }

    @Test
    fun aDeviceThatProvedNothingIsNeverTrustedByWhatItSays() {
        // (No certificate presented: the fingerprint it names in its offer counts for nothing.)
        assertEquals(Lan.Decision.Ask(canTrust = false), Lan.decide(null, paired, trusted, Lan.Others.Ask))
        assertEquals(Lan.Decision.Refuse, Lan.decide(null, paired, trusted, Lan.Others.Refuse))
    }

    // ---- What devices say of themselves ----

    @Test
    fun thisPhoneSaysItIsAGlance() {
        val me = Lan.Info("Pixel", "Android", "mobile", "AB".repeat(32), 53317, https = true, announce = true, glance = true)
        val json = me.toJson()
        assertEquals("2.1", json.getString("version"))
        assertEquals("https", json.getString("protocol"))
        assertEquals(53317, json.getInt("port"))
        assertTrue(json.getBoolean("glance"))
        assertTrue(json.getBoolean("announce"))
        assertTrue(json.getBoolean("announcement"))
        assertFalse(json.getBoolean("download"))
        assertEquals(me, Lan.Info.of(json.toString()))
        val plain = me.copy(announce = false).toJson()
        assertFalse(plain.has("announce"))
        assertFalse(plain.has("announcement"))
        assertTrue(plain.getBoolean("glance"))
    }

    @Test
    fun anotherLocalSendIsReadAsItIs() {
        val info = Lan.Info.of("""{"alias":"Nice Orange","version":"2.0","deviceModel":"Samsung","deviceType":"MOBILE","fingerprint":"xyz","port":53318,"protocol":"http","download":true,"announcement":true}""")
        assertNotNull(info)
        info!!
        assertEquals("Nice Orange", info.alias)
        assertEquals("Samsung", info.deviceModel)
        assertEquals("mobile", info.deviceType)
        assertEquals(53318, info.port)
        assertFalse(info.https)
        assertTrue(info.announce)
        assertFalse(info.glance)
    }

    @Test
    fun aDeviceIsReadOnlyAsFarAsItCanBe() {
        // An answer to register names no port nor protocol (it is answered over what it was asked).
        val answered = Lan.Info.of("""{"alias":"PC","deviceModel":null,"deviceType":null,"fingerprint":"F"}""")!!
        assertNull(answered.port)
        assertNull(answered.deviceModel)
        assertTrue(answered.https)
        assertNull(Lan.Info.of("""{"alias":"PC"}"""))
        assertNull(Lan.Info.of("""{"alias":"PC","fingerprint":""}"""))
        assertNull(Lan.Info.of("""{"alias":"PC","fingerprint":"${"F".repeat(129)}"}"""))
        assertNull(Lan.Info.of("not json"))
        assertNull(Lan.Info.of("""{"alias":"PC","fingerprint":"F","port":70000}""")!!.port)
        assertEquals("LocalSend", Lan.Info.of("""{"alias":"\u0001","fingerprint":"F"}""")!!.alias)
        assertEquals("ab", Lan.Info.of("""{"alias":" a\nb ","fingerprint":"F"}""")!!.alias)
        assertFalse(Lan.Info.of("""{"fingerprint":"F","glance":"yes"}""")!!.glance)
    }

    // ---- What is offered ----

    private fun offer(files: JSONObject) = JSONObject().put("info", JSONObject().put("alias", "PC").put("fingerprint", "F")).put("files", files).toString()

    private fun file(id: String, name: String, size: Any?, type: String = "image/jpeg") = JSONObject().put("id", id).put("fileName", name).put("size", size ?: JSONObject.NULL).put("fileType", type)

    @Test
    fun anOfferIsReadWithItsFiles() {
        val read = Lan.Offer.of(offer(JSONObject().put("a", file("a", "x.jpg", 10).put("sha256", "AB".repeat(32))).put("b", file("b", "y.jpg", 20))))
        assertEquals(2, read.files.size)
        assertEquals(30L, read.total)
        assertNull(read.words)
        assertEquals("ab".repeat(32), read.files.single { it.id == "a" }.sha256)
        assertNull(read.files.single { it.id == "b" }.sha256)
    }

    @Test
    fun aMessageIsOneTextWithWords() {
        val words = Lan.Offer.of(offer(JSONObject().put("m", file("m", "m.txt", 5, "text/plain").put("preview", "hello"))))
        assertEquals("hello", words.words)
        // (A text file with no words shown, or a text among other files, is a file.)
        assertNull(Lan.Offer.of(offer(JSONObject().put("m", file("m", "m.txt", 5, "text/plain")))).words)
        assertNull(Lan.Offer.of(offer(JSONObject().put("m", file("m", "m.txt", 5, "text/plain").put("preview", "  ")))).words)
        assertNull(Lan.Offer.of(offer(JSONObject().put("m", file("m", "m.txt", 5, "text/plain").put("preview", "hi")).put("n", file("n", "n.jpg", 1)))).words)
    }

    private fun refused(text: String) {
        try {
            Lan.Offer.of(text)
            fail("taken: $text")
        } catch (bad: Lan.Bad) {
            assertEquals(400, bad.status)
        }
    }

    @Test
    fun anOfferThatIsNoneIsRefused() {
        refused("nothing")
        refused(JSONObject().put("files", JSONObject()).toString())
        refused(JSONObject().put("info", JSONObject().put("alias", "PC").put("fingerprint", "F")).toString())
        refused(offer(JSONObject().put("a", "file")))
        refused(offer(JSONObject().put("a", file("a", "x.jpg", null))))
        refused(offer(JSONObject().put("a", file("a", "x.jpg", "10"))))
        refused(offer(JSONObject().put("a", file("a", "x.jpg", -1))))
        refused(offer(JSONObject().put("i".repeat(201), file("i", "x.jpg", 1))))
        refused(offer(JSONObject().put("a", file("a", "x".repeat(4097), 1))))
        refused(offer(JSONObject().put("a", file("a", "x.jpg", Long.MAX_VALUE)).put("b", file("b", "y.jpg", Long.MAX_VALUE))))
        val many = JSONObject()
        for (at in 0..Lan.MOST_FILES) many.put("f$at", file("f$at", "x.jpg", 1))
        refused(offer(many))
    }

    // ---- HTTP ----

    private fun request(text: String) = Lan.request(ByteArrayInputStream(text.toByteArray()))

    @Test
    fun aRequestIsReadWithItsQueryAndBody() {
        val body = """{"a":1}"""
        val read = request("POST /api/localsend/v2/upload?sessionId=s%201&fileId=a+b&token=t HTTP/1.1\r\nHost: x\r\nContent-Length: ${body.length}\r\nContent-Type: application/json\r\n\r\n$body" + "after")!!
        assertEquals("POST", read.method)
        assertEquals("/api/localsend/v2/upload", read.path)
        assertEquals(mapOf("sessionId" to "s 1", "fileId" to "a b", "token" to "t"), read.query)
        assertEquals("application/json", read.headers["content-type"])
        assertEquals(body, read.text(100))
    }

    @Test
    fun aChunkedBodyIsReadWhole() {
        val read = request("POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n6;ext=1\r\npedia \r\nE\r\nin \r\n\r\nchunks.\r\n0\r\n\r\n")!!
        assertEquals("Wikipedia in \r\n\r\nchunks.", read.text(1000))
    }

    @Test
    fun aRequestWithNoBodyHasNone() {
        val read = request("GET /api/localsend/v2/info HTTP/1.1\r\nHost: x\r\n\r\n")!!
        assertEquals("", read.text(10))
        assertTrue(read.query.isEmpty())
    }

    @Test
    fun aBodyTooLargeIsRefused() {
        val read = request("POST /x HTTP/1.1\r\nContent-Length: 20\r\n\r\n" + "a".repeat(20))!!
        try {
            read.text(10)
            fail()
        } catch (bad: Lan.Bad) {
            assertEquals(413, bad.status)
        }
    }

    @Test
    fun aConnectionClosedAtOnceHasNoRequest() {
        assertNull(request(""))
    }

    @Test
    fun badRequestsAreSaidToBeBad() {
        for ((text, status) in listOf(
            "GET /x HTTP/1.1\r\nHost" to 400,
            "GET\r\n\r\n" to 400,
            "POST /x HTTP/1.1\r\nContent-Length: many\r\n\r\n" to 400,
            "GET /x HTTP/1.1\r\n" + "X: ${"a".repeat(17_000)}\r\n\r\n" to 431,
        )) {
            try {
                request(text)
                fail(text.take(40))
            } catch (bad: Lan.Bad) {
                assertEquals(status, bad.status)
            }
        }
        val chunk = request("POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n")!!
        try {
            chunk.text(100)
            fail()
        } catch (bad: Lan.Bad) {
            assertEquals(400, bad.status)
        }
    }

    @Test
    fun anAnswerSaysItsLengthAndCloses() {
        val out = ByteArrayOutputStream()
        Lan.answer(out, 200, """{"sessionId":"s"}""")
        val said = out.toString(Charsets.UTF_8.name())
        assertTrue(said.startsWith("HTTP/1.1 200 "))
        assertTrue(said.contains("Content-Type: application/json\r\n"))
        assertTrue(said.contains("Content-Length: 17\r\n"))
        assertTrue(said.contains("Connection: close\r\n"))
        assertTrue(said.endsWith("\r\n\r\n{\"sessionId\":\"s\"}"))
        val none = ByteArrayOutputStream().also { Lan.answer(it, 204, null) }.toString(Charsets.UTF_8.name())
        assertTrue(none.contains("Content-Length: 0\r\n"))
        assertFalse(none.contains("Content-Type"))
    }
}
