package org.openchromecast.receiver;

import org.junit.Test;
import static org.junit.Assert.*;

public class SystemVolumePolicyTest {
    @Test public void waitsForAsynchronousDeviceInitialization() {
        assertFalse(SystemVolumePolicy.ready(0, 0));
        assertTrue(SystemVolumePolicy.ready(0, 15));
        assertFalse(SystemVolumePolicy.ready(5, 2));
        assertFalse(SystemVolumePolicy.ready(-1, 15));
    }

    @Test public void castRequestsUseActualDeviceSteps() {
        assertEquals(0, SystemVolumePolicy.castIndex(0f, 0, 15));
        assertEquals(5, SystemVolumePolicy.castIndex(0.35f, 0, 15));
        assertEquals(15, SystemVolumePolicy.castIndex(1f, 0, 15));
        assertEquals(0, SystemVolumePolicy.castIndex(-1f, 0, 15));
        assertEquals(15, SystemVolumePolicy.castIndex(2f, 0, 15));
    }

    @Test public void deviceReadbackReportsQuantizedNotRequestedVolume() {
        assertEquals(1f / 3f, SystemVolumePolicy.level(5, 0, 15), 0.00001f);
        assertEquals(1f / 15f, SystemVolumePolicy.step(0, 15), 0.00001f);
        assertEquals(0f, SystemVolumePolicy.level(-1, 0, 15), 0f);
        assertEquals(1f, SystemVolumePolicy.level(16, 0, 15), 0f);
    }

    @Test public void migrationNeverRaisesTheCurrentDeviceIndex() {
        for (int index = 0; index <= 15; index++) {
            for (float gain : new float[]{0f, 0.15f, 0.3f, 0.5f, 1f, 2f, Float.NaN}) {
                assertTrue(SystemVolumePolicy.migrationIndex(index, gain, 0, 15) <= index);
            }
        }
        assertEquals(4, SystemVolumePolicy.migrationIndex(15, 0.3f, 0, 15));
        assertEquals(0, SystemVolumePolicy.migrationIndex(15, 0f, 0, 15));
        assertEquals(5, SystemVolumePolicy.migrationIndex(5, 1f, 0, 15));
    }

    @Test public void nonzeroDeviceMinimumIsRespected() {
        assertEquals(2, SystemVolumePolicy.castIndex(0f, 2, 12));
        assertEquals(7, SystemVolumePolicy.castIndex(0.5f, 2, 12));
        assertEquals(4, SystemVolumePolicy.migrationIndex(10, 0.3f, 2, 12));
        assertEquals(0.5f, SystemVolumePolicy.level(7, 2, 12), 0f);
    }

    @Test(expected = IllegalArgumentException.class) public void rejectsNonFiniteRequests() {
        SystemVolumePolicy.castIndex(Float.NaN, 0, 15);
    }

    @Test(expected = IllegalArgumentException.class) public void refusesFixedVolumeInsteadOfPretendingToControlIt() {
        SystemVolumePolicy.level(0, 0, 0);
    }
}
