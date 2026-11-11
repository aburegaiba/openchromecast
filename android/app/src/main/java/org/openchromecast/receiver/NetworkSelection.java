package org.openchromecast.receiver;

final class NetworkSelection {
    private Object current;

    void available(Object network) { current = network; }

    boolean isCurrent(Object network) { return network != null && network.equals(current); }

    boolean lost(Object network) {
        if (!isCurrent(network)) return false;
        current = null;
        return true;
    }
}
