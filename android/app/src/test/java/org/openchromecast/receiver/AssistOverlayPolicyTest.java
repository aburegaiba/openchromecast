package org.openchromecast.receiver;

import org.junit.Test;
import static org.junit.Assert.*;

public class AssistOverlayPolicyTest {
    @Test public void resumesOnlyAnOptedInPermittedShortcut() {
        for (String action : new String[]{"android.intent.action.BOOT_COMPLETED", "android.intent.action.MY_PACKAGE_REPLACED"}) {
            assertTrue(AssistOverlayPolicy.shouldResume(action, true, true));
            assertFalse(AssistOverlayPolicy.shouldResume(action, false, true));
            assertFalse(AssistOverlayPolicy.shouldResume(action, true, false));
        }
        assertFalse(AssistOverlayPolicy.shouldResume(null, true, true));
        assertFalse(AssistOverlayPolicy.shouldResume("untrusted", true, true));
    }

    @Test public void launchesOnlyKnownCompanionPackages() {
        assertEquals(AssistOverlayPolicy.FULL_PACKAGE, AssistOverlayPolicy.companionPackage(true, true));
        assertEquals(AssistOverlayPolicy.MINIMAL_PACKAGE, AssistOverlayPolicy.companionPackage(false, true));
        assertNull(AssistOverlayPolicy.companionPackage(false, false));
    }

    @Test public void corruptOrOutOfRangeFractionsStayOnScreen() {
        assertEquals(0.45f, AssistOverlayPolicy.fraction(Float.NaN), 0f);
        assertEquals(0.45f, AssistOverlayPolicy.fraction(Float.POSITIVE_INFINITY), 0f);
        assertEquals(0f, AssistOverlayPolicy.fraction(-2f), 0f);
        assertEquals(1f, AssistOverlayPolicy.fraction(2f), 0f);
    }

    @Test public void clampingHandlesBothEdgesAndSmallScreens() {
        assertEquals(8, AssistOverlayPolicy.clamp(-100, 8, 100));
        assertEquals(100, AssistOverlayPolicy.clamp(200, 8, 100));
        assertEquals(8, AssistOverlayPolicy.clamp(0, 8, -20));
    }

    @Test public void positionRoundTripsAcrossDisplaySizes() {
        for (int bottom : new int[]{400, 720, 1200}) {
            int position = AssistOverlayPolicy.verticalPosition(32, bottom, 0.6f);
            assertTrue(position >= 32 && position <= bottom);
            assertEquals(0.6f, AssistOverlayPolicy.verticalFraction(position, 32, bottom), 0.003f);
        }
        assertEquals(32, AssistOverlayPolicy.verticalPosition(32, 20, 1f));
        assertEquals(0.45f, AssistOverlayPolicy.verticalFraction(32, 32, 32), 0f);
    }
}
