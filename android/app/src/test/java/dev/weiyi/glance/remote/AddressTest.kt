package dev.weiyi.glance.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AddressTest {
    private fun read(typed: String) = Address.parse(typed)?.let { (address, port) -> address.hostAddress to port }

    /** An address is read as typed: IPv4 or IPv6, with a port or not, in brackets or not, spaces around it aside. */
    @Test
    fun anAddressIsReadAsTyped() {
        assertEquals("10.21.140.12" to null, read("10.21.140.12"))
        assertEquals("10.21.140.12" to 53318, read(" 10.21.140.12:53318 "))
        assertEquals("2001:da8:215:4a07:0:0:0:1c" to null, read("2001:da8:215:4a07::1c"))
        assertEquals("2001:da8:0:0:0:0:0:1" to null, read("[2001:da8::1]"))
        assertEquals("2001:da8:0:0:0:0:0:1" to 53318, read("[2001:da8::1]:53318"))
    }

    /** What is no address (a name, an address cut short, a number too large, a port 0) is none, and no name is looked up. */
    @Test
    fun whatIsNoAddressIsNone() {
        for (typed in listOf("", "实验室电脑", "lab-pc", "10.21.140", "10.21.140.256", "10.21.140.12:0", "10.21.140.12:70000", "[2001:da8::1", "[2001:da8::1]x", "2001:da8::zz", "localhost")) {
            assertNull(typed, Address.parse(typed))
        }
    }

    /** An address told as "host:port" is split (an IPv6 host in brackets); one is shown without its port where that is one looked on for an address alone. */
    @Test
    fun anAddressToldIsSplitAndShown() {
        assertEquals("10.0.0.2" to 53317, Address.split("10.0.0.2:53317"))
        assertEquals("2001:da8::1" to 53320, Address.split("[2001:da8::1]:53320"))
        assertNull(Address.split("10.0.0.2"))
        assertEquals("[2001:da8::1]:53320", Address.at("2001:da8::1", 53320))
        assertEquals("2001:da8::1", Address.shown("[2001:da8::1]:53320"))
        assertEquals("10.0.0.2", Address.shown("10.0.0.2:53317"))
        assertEquals("10.0.0.2:53318", Address.shown("10.0.0.2:53318"))
    }

    /** Where else a Glance device is reached is read as it tells it, and told as this phone tells it; what is no address is passed over. */
    @Test
    fun theAddressesADeviceTellsAreRead() {
        val info = Lan.Info.of("""{"alias":"实验室电脑","fingerprint":"AB","port":53320,"protocol":"https","glance":true,"addresses":["[2001:da8::1c]:53320","10.21.140.12:53320","nonsense"]}""")!!
        assertEquals(listOf("[2001:da8::1c]:53320", "10.21.140.12:53320"), info.addresses)
        assertEquals(info.addresses, Lan.Info.of(info.toJson().toString())!!.addresses)
        assertEquals(emptyList<String>(), Lan.Info.of("""{"alias":"Pixel","fingerprint":"CD"}""")!!.addresses)
    }
}
