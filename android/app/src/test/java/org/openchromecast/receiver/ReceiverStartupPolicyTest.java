package org.openchromecast.receiver;

import org.junit.Test;
import static org.junit.Assert.*;

public class ReceiverStartupPolicyTest {
    @Test public void resumesOnlyAfterAnExplicitOptInAndStart() {
        String boot = "android.intent.action.BOOT_COMPLETED";
        assertTrue(ReceiverStartupPolicy.shouldResume(boot, true, true));
        assertFalse(ReceiverStartupPolicy.shouldResume(boot, false, true));
        assertFalse(ReceiverStartupPolicy.shouldResume(boot, true, false));
        assertFalse(ReceiverStartupPolicy.shouldResume(boot, false, false));
    }

    @Test public void ignoresOtherOrMissingActions() {
        assertFalse(ReceiverStartupPolicy.shouldResume(null, true, true));
        assertFalse(ReceiverStartupPolicy.shouldResume("android.intent.action.LOCKED_BOOT_COMPLETED", true, true));
        assertFalse(ReceiverStartupPolicy.shouldResume("example.invalid.START", true, true));
    }
}
