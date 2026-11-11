//! Device identity: RSA 2048 key + X.509 certificates.
//!
//! Reverse-engineered facts from Chromium's Cast validator
//! (`cast_auth_util.cc` / `cast_cert_validator.cc`) that drive this design:
//!
//! * The Cast device-auth signature is **RSASSA-PKCS#1 v1.5** (with SHA-256);
//!   the validator's `VerifySignatureOverData` requires `EVP_PKEY_RSA`.
//! * The signature covers **`sender_nonce || peer_cert_der`** (the TLS cert),
//!   not the nonce alone.
//! * The TLS certificate is *not* chain-verified; it only needs a valid date
//!   window and a remaining lifetime of **at most 4 days**
//!   (`kMaxSelfSignedCertLifetimeInDays`). Real receivers rotate it often.
//! * The `client_auth_certificate` chain, on the other hand, must path-build
//!   to one of the two **built-in Cast trust anchors** — that is the hard wall
//!   (see `docs/reverse-engineering.md`).
//!
//! Default mode generates a single short-lived self-signed RSA identity used
//! for both TLS and device auth (fine for senders that skip chain validation,
//! e.g. `pychromecast`). Real-device mode loads a Google-issued device cert +
//! RSA key for device auth, and generates a separate short-lived TLS cert.

use anyhow::{Context, Result, bail};
use rsa::RsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use rsa::rand_core::OsRng;
use rsa::sha2::Sha256;
use rsa::signature::{SignatureEncoding, Signer};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use std::io::Cursor;
use std::path::Path;
use time::{Duration, OffsetDateTime};
use tracing::info;

/// TLS certs must not have more than 4 days of remaining lifetime to pass
/// Chromium's `VerifyTLSCertificate` check.
const TLS_CERT_LIFETIME_DAYS: i64 = 3;
const TLS_RENEWAL_MARGIN_HOURS: i64 = 24;

struct TlsMaterial {
    key_der: Vec<u8>,
    cert_der: Vec<u8>,
    not_before: OffsetDateTime,
    not_after: OffsetDateTime,
}

/// Signing key + certificate material for TLS and Cast device auth.
pub struct Identity {
    signer: SigningKey<Sha256>,
    /// `client_auth_certificate` (must chain to a Cast root CA in real devices).
    auth_cert_der: Vec<u8>,
    /// Certificates after the leaf, preserved in the supplied PEM order.
    intermediate_ders: Vec<Vec<u8>>,
    tls: TlsMaterial,
    generated: bool,
}

impl Identity {
    /// Load credentials from PEM files, or generate a fresh self-signed identity.
    pub fn load_or_generate(cert_pem: Option<&Path>, key_pem: Option<&Path>) -> Result<Self> {
        match (cert_pem, key_pem) {
            (Some(c), Some(k)) => Self::load_real_device(c, k),
            (None, None) => {
                info!("no device credentials provided; generating a self-signed identity");
                Self::generate()
            }
            _ => bail!("provide both --cert and --key together, or neither"),
        }
    }

    /// Generate one RSA 2048 key + a short-lived self-signed certificate used
    /// for both TLS and device auth.
    pub fn generate() -> Result<Self> {
        Self::generate_at(OffsetDateTime::now_utc())
    }

    fn generate_at(now: OffsetDateTime) -> Result<Self> {
        let tls = generate_rsa_cert_short_lived("OpenChromecast", now)?;
        let signer = rsa_signer(&tls.key_der)?;
        Ok(Self {
            signer,
            auth_cert_der: tls.cert_der.clone(),
            intermediate_ders: Vec::new(),
            tls,
            generated: true,
        })
    }

    /// Load a real device certificate + RSA private key for device auth, and
    /// generate a separate short-lived TLS certificate.
    pub fn load_real_device(cert_path: &Path, key_path: &Path) -> Result<Self> {
        let cert_pem = std::fs::read_to_string(cert_path)?;
        let key_pem = std::fs::read_to_string(key_path)?;
        let identity = Self::from_device_pem(&cert_pem, &key_pem)?;
        info!("loaded device credentials from {cert_path:?}");
        Ok(identity)
    }

    /// Parse a supplied identity. Key consistency is not a Cast trust-chain check.
    pub fn from_device_pem(cert_pem: &str, key_pem: &str) -> Result<Self> {
        let certs = rustls_pemfile::certs(&mut Cursor::new(cert_pem.as_bytes()))
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to parse certificate PEM")?;
        let auth_cert_der = certs
            .first()
            .context("no certificate found in PEM")?
            .as_ref()
            .to_vec();
        let intermediate_ders = certs.iter().skip(1).map(|c| c.as_ref().to_vec()).collect();

        // Real Chromecast keys are RSA (PKCS#8 "PRIVATE KEY" or PKCS#1
        // "RSA PRIVATE KEY").
        let key = RsaPrivateKey::from_pkcs8_pem(key_pem)
            .or_else(|_| rsa::pkcs1::DecodeRsaPrivateKey::from_pkcs1_pem(key_pem))
            .context("failed to parse device private key (expected RSA PKCS#8/PKCS#1 PEM)")?;
        let pkcs8 = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            key.to_pkcs8_der()?.as_bytes().to_vec(),
        ));
        let signing_key = rustls::crypto::ring::sign::any_supported_type(&pkcs8)
            .context("device key is not supported by TLS")?;
        rustls::sign::CertifiedKey::new(vec![certs[0].clone()], signing_key)
            .keys_match()
            .context("device certificate and key do not match")?;

        // The real auth cert is long-lived, which would fail the 4-day TLS
        // check — so use a fresh short-lived TLS cert instead.
        let tls = generate_rsa_cert_short_lived("OpenChromecast", OffsetDateTime::now_utc())?;

        Ok(Self {
            signer: SigningKey::new(key),
            auth_cert_der,
            intermediate_ders,
            tls,
            generated: false,
        })
    }

    /// RSASSA-PKCS#1 v1.5 with SHA-256 over `data` — the Cast device-auth
    /// signature. The caller must pass `sender_nonce || tls_cert_der`.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        self.signer.sign(data).to_vec()
    }

    pub fn auth_cert_der(&self) -> &[u8] {
        &self.auth_cert_der
    }

    pub fn intermediate_ders(&self) -> &[Vec<u8>] {
        &self.intermediate_ders
    }

    pub fn tls_cert_der(&self) -> &[u8] {
        &self.tls.cert_der
    }

    /// The TLS private key in PKCS#8 DER form, for the rustls TLS server.
    pub fn tls_private_key_der(&self) -> Result<PrivateKeyDer<'static>> {
        Ok(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            self.tls.key_der.clone(),
        )))
    }

    pub fn tls_valid_at(&self, now: OffsetDateTime) -> bool {
        now >= self.tls.not_before && now < self.tls.not_after
    }

    pub fn tls_renewal_due_at(&self, now: OffsetDateTime) -> bool {
        !self.tls_valid_at(now)
            || now + Duration::hours(TLS_RENEWAL_MARGIN_HOURS) >= self.tls.not_after
    }

    /// Retained connections keep their old snapshot. A supplied auth key/chain is preserved.
    pub fn renew_tls_at(&self, now: OffsetDateTime) -> Result<Self> {
        if self.generated {
            return Self::generate_at(now);
        }
        Ok(Self {
            signer: self.signer.clone(),
            auth_cert_der: self.auth_cert_der.clone(),
            intermediate_ders: self.intermediate_ders.clone(),
            tls: generate_rsa_cert_short_lived("OpenChromecast", now)?,
            generated: false,
        })
    }
}

fn rsa_signer(key_der: &[u8]) -> Result<SigningKey<Sha256>> {
    let key = RsaPrivateKey::from_pkcs8_der(key_der).context("failed to import RSA key")?;
    Ok(SigningKey::new(key))
}

/// Generate an RSA 2048 key + self-signed cert that is valid for only a few
/// days (passes the Cast TLS lifetime check), including its validity window.
fn generate_rsa_cert_short_lived(cn: &str, now: OffsetDateTime) -> Result<TlsMaterial> {
    // rcgen's ring backend cannot generate RSA keys, so generate the key with
    // the pure-Rust `rsa` crate and import it for certificate building.
    let key = RsaPrivateKey::new(&mut OsRng, 2048).context("failed to generate RSA key")?;
    let key_der = key.to_pkcs8_der()?.as_bytes().to_vec();
    let key_pem = key.to_pkcs8_pem(LineEnding::LF)?;
    let key_pair =
        rcgen::KeyPair::from_pem(&key_pem).context("failed to import RSA key into rcgen")?;

    let mut params = rcgen::CertificateParams::new(Vec::new())?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, cn);
    params.is_ca = rcgen::IsCa::NoCa;
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::DigitalSignature,
        rcgen::KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    // Short-lived so it passes Chromium's 4-day TLS-cert lifetime check.
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(TLS_CERT_LIFETIME_DAYS);
    let cert = params
        .self_signed(&key_pair)
        .context("failed to self-sign certificate")?;
    Ok(TlsMaterial {
        key_der,
        cert_der: cert.der().to_vec(),
        not_before: now - Duration::days(1),
        not_after: now + Duration::days(TLS_CERT_LIFETIME_DAYS),
    })
}

#[cfg(test)]
mod rotation_tests {
    use super::*;

    #[test]
    fn renewal_policy_covers_margin_expiry_and_clock_jumps() {
        let now = OffsetDateTime::now_utc();
        let identity = Identity::generate_at(now).unwrap();
        assert!(identity.tls_valid_at(now));
        assert!(!identity.tls_renewal_due_at(now));
        assert!(!identity.tls_renewal_due_at(now + Duration::hours(47)));
        assert!(identity.tls_renewal_due_at(now + Duration::hours(48)));
        assert!(!identity.tls_valid_at(now + Duration::days(3)));
        assert!(identity.tls_renewal_due_at(now + Duration::days(3)));
        assert!(!identity.tls_valid_at(now - Duration::days(2)));
        assert!(identity.tls_renewal_due_at(now - Duration::days(2)));
    }

    #[test]
    fn generated_identity_refreshes_both_ephemeral_certificates() {
        let now = OffsetDateTime::now_utc();
        let identity = Identity::generate_at(now).unwrap();
        let renewed = identity.renew_tls_at(now + Duration::days(2)).unwrap();
        assert_ne!(identity.tls_cert_der(), renewed.tls_cert_der());
        assert_eq!(renewed.auth_cert_der(), renewed.tls_cert_der());
        assert!(renewed.tls_valid_at(now + Duration::days(4)));
        assert!(!renewed.tls_renewal_due_at(now + Duration::days(2)));
    }

    #[test]
    fn provided_identity_refresh_keeps_auth_key_and_entire_chain() {
        let now = OffsetDateTime::now_utc();
        let mut identity = Identity::generate_at(now).unwrap();
        identity.generated = false;
        identity.intermediate_ders = vec![vec![1, 2], vec![3, 4]];
        let renewed = identity.renew_tls_at(now + Duration::days(2)).unwrap();
        assert_ne!(identity.tls_cert_der(), renewed.tls_cert_der());
        assert_eq!(identity.auth_cert_der(), renewed.auth_cert_der());
        assert_eq!(identity.intermediate_ders(), renewed.intermediate_ders());
        let mut challenge = vec![0x45; 32];
        challenge.extend_from_slice(renewed.tls_cert_der());
        assert_eq!(identity.sign(&challenge), renewed.sign(&challenge));
    }
}
