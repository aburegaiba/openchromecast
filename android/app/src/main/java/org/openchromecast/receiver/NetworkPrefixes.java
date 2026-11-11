package org.openchromecast.receiver;

import java.net.Inet6Address;
import java.net.InetAddress;
import java.net.UnknownHostException;

final class NetworkPrefixes {
    private NetworkPrefixes() {}

    static String format(InetAddress address, int prefixLength) {
        if (!(address instanceof Inet6Address) || prefixLength < 64 || prefixLength > 128
                || address.isAnyLocalAddress() || address.isLoopbackAddress()
                || address.isLinkLocalAddress() || address.isMulticastAddress()) return null;
        try {
            InetAddress plain = InetAddress.getByAddress(address.getAddress());
            return plain instanceof Inet6Address ? plain.getHostAddress() + "/" + prefixLength : null;
        } catch (UnknownHostException ignored) {
            return null;
        }
    }
}
