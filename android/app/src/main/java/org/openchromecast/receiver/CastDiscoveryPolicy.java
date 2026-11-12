package org.openchromecast.receiver;

final class CastDiscoveryPolicy {
    static final int CAPABILITIES = 5; // Video and audio output only.
    static final String REGISTRATION_TYPE = "_googlecast._tcp,_CC1AD845,_233637DE";

    private CastDiscoveryPolicy() {}
}
