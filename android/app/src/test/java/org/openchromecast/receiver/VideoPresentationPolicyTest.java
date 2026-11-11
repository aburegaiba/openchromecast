package org.openchromecast.receiver;

import org.junit.Test;
import static org.junit.Assert.*;

public class VideoPresentationPolicyTest {
    @Test public void audioNeverTakesOverAnotherApplication() {
        for (int flags = 0; flags < 16; flags++) {
            assertFalse(VideoPresentationPolicy.shouldPresent(false, (flags & 1) != 0,
                (flags & 2) != 0, (flags & 4) != 0, (flags & 8) != 0));
        }
    }

    @Test public void visibleControlsKeepExistingVideoBehaviorWithoutPermission() {
        assertTrue(VideoPresentationPolicy.shouldPresent(true, false, true, false, false));
    }

    @Test public void backgroundVideoRequiresBothOptInAndSystemPermission() {
        assertFalse(VideoPresentationPolicy.shouldPresent(true, false, false, false, true));
        assertFalse(VideoPresentationPolicy.shouldPresent(true, false, false, true, false));
        assertFalse(VideoPresentationPolicy.shouldPresent(true, false, false, false, false));
        assertTrue(VideoPresentationPolicy.shouldPresent(true, false, false, true, true));
    }

    @Test public void dismissedOrAlreadyPresentedVideoDoesNotReopenOnTrackUpdates() {
        for (int flags = 0; flags < 8; flags++) {
            assertFalse(VideoPresentationPolicy.shouldPresent(true, true,
                (flags & 1) != 0, (flags & 2) != 0, (flags & 4) != 0));
        }
    }
}
