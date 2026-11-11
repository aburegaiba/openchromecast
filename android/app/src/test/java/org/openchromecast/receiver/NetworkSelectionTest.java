package org.openchromecast.receiver;

import static org.junit.Assert.*;
import org.junit.Test;

public class NetworkSelectionTest {
    @Test public void propertiesMustBelongToCurrentNetwork() {
        NetworkSelection selection = new NetworkSelection();
        selection.available("wifi-one");
        assertTrue(selection.isCurrent(new String("wifi-one")));
        assertFalse(selection.isCurrent("wifi-two"));
    }

    @Test public void lateLossOfPreviousNetworkDoesNotRemoveNewNetwork() {
        NetworkSelection selection = new NetworkSelection();
        selection.available("wifi-one");
        selection.available("wifi-two");
        assertFalse(selection.lost("wifi-one"));
        assertTrue(selection.isCurrent("wifi-two"));
    }

    @Test public void losingCurrentNetworkRevokesItImmediately() {
        NetworkSelection selection = new NetworkSelection();
        selection.available("wifi-one");
        assertTrue(selection.lost("wifi-one"));
        assertFalse(selection.isCurrent("wifi-one"));
        assertFalse(selection.lost("wifi-one"));
    }

    @Test public void missingNetworkNeverMatches() {
        NetworkSelection selection = new NetworkSelection();
        selection.available(null);
        assertFalse(selection.isCurrent(null));
        assertFalse(selection.lost(null));
    }
}
