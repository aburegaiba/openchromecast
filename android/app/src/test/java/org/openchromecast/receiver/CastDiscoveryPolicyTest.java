package org.openchromecast.receiver;

import static org.junit.Assert.*;
import org.junit.Test;

public class CastDiscoveryPolicyTest {
    @Test public void registersTwoSupportedSubtypesOnOneBaseInstance() {
        String[] types = CastDiscoveryPolicy.REGISTRATION_TYPE.split(",");
        assertArrayEquals(new String[]{"_googlecast._tcp", "_CC1AD845", "_233637DE"}, types);
        assertFalse(CastDiscoveryPolicy.REGISTRATION_TYPE.contains(" "));
        assertFalse(CastDiscoveryPolicy.REGISTRATION_TYPE.contains("._sub."));
    }

    @Test public void doesNotClaimInputOrGroupCapabilities() {
        assertEquals(5, CastDiscoveryPolicy.CAPABILITIES);
    }
}
