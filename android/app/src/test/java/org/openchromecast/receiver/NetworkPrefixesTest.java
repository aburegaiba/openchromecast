package org.openchromecast.receiver;

import static org.junit.Assert.*;
import java.net.Inet6Address;
import java.net.InetAddress;
import org.junit.Test;

public class NetworkPrefixesTest {
    @Test public void formatsUnicastGlobalAndUniqueLocalNetworks() throws Exception {
        for (String address : new String[]{"2001:db8:1::20", "fd00:2::20"}) {
            assertTrue(NetworkPrefixes.format(InetAddress.getByName(address), 64).endsWith("/64"));
        }
    }

    @Test public void stripsInterfaceScopeWithoutStringParsing() throws Exception {
        Inet6Address scoped = Inet6Address.getByAddress(null,
            InetAddress.getByName("2001:db8:1::20").getAddress(), 7);
        assertFalse(NetworkPrefixes.format(scoped, 64).contains("%"));
    }

    @Test public void rejectsNonPrefixLocalOrSpecialAddresses() throws Exception {
        for (String address : new String[]{"192.0.2.1", "::", "::1", "fe80::20", "ff02::1"}) {
            assertNull(NetworkPrefixes.format(InetAddress.getByName(address), 64));
        }
    }

    @Test public void rejectsBroadOrInvalidMasks() throws Exception {
        InetAddress address = InetAddress.getByName("2001:db8:1::20");
        for (int bits : new int[]{-1, 0, 32, 63, 129}) assertNull(NetworkPrefixes.format(address, bits));
        assertTrue(NetworkPrefixes.format(address, 128).endsWith("/128"));
    }
}
