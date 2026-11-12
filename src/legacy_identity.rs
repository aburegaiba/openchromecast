use anyhow::{Context, Result, ensure};
use rsa::RsaPublicKey;
use rsa::pkcs8::DecodePublicKey;
use rsa::signature::Verifier;
use std::sync::Arc;
use time::OffsetDateTime;
use vibecast_security::{CertificateBundle, CertificateStore};

pub(super) struct BundleSet {
    bundles: Vec<CertificateBundle>,
}

impl BundleSet {
    pub(super) fn load(json: &str) -> Result<Arc<Self>> {
        ensure!(json.len() <= 32 * 1024 * 1024, "Cast bundle exceeds limit");
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|_| anyhow::anyhow!("invalid Cast bundle JSON"))?;
        let windows = value
            .get("certs")
            .and_then(|v| v.as_array())
            .context("missing Cast windows")?;
        ensure!(
            (1..=4096).contains(&windows.len()),
            "invalid Cast window count"
        );
        for value in
            [value.get("cpu"), value.get("ica")]
                .into_iter()
                .chain(windows.iter().flat_map(|w| {
                    [
                        w.get("pu"),
                        w.get("pr"),
                        w.get("sig_sha1"),
                        w.get("sig_sha256"),
                    ]
                }))
        {
            ensure!(
                value
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| (1..=65536).contains(&s.len())),
                "invalid Cast bundle field"
            );
        }
        let store = CertificateStore::from_manifest_str(json)
            .map_err(|_| anyhow::anyhow!("invalid Cast bundle or no currently valid window"))?;
        for bundle in store.bundles() {
            ensure!(
                bundle.not_valid_after > bundle.not_valid_before
                    && bundle.not_valid_after - bundle.not_valid_before <= 4 * 86400,
                "invalid Cast TLS validity window"
            );
            // Verify cached signatures; the bundle does not supply the device's signing key.
            let spki = bundle
                .device_public_key_der()
                .map_err(|_| anyhow::anyhow!("invalid Cast device certificate"))?;
            let key =
                RsaPublicKey::from_public_key_der(&spki).context("Cast device key is not RSA")?;
            let signature = |bytes: &[u8]| {
                rsa::pkcs1v15::Signature::try_from(bytes)
                    .map_err(|_| anyhow::anyhow!("invalid cached Cast signature"))
            };
            rsa::pkcs1v15::VerifyingKey::<rsa::sha2::Sha256>::new(key.clone())
                .verify(&bundle.peer_cert_der, &signature(&bundle.signature_sha256)?)
                .context("cached SHA256 signature does not match TLS certificate")?;
            rsa::pkcs1v15::VerifyingKey::<sha1::Sha1>::new(key)
                .verify(&bundle.peer_cert_der, &signature(&bundle.signature_sha1)?)
                .context("cached SHA1 signature does not match TLS certificate")?;
            super::Identity::bundle_tls(bundle)?;
        }
        Ok(Arc::new(Self {
            bundles: store.bundles().to_vec(),
        }))
    }

    pub(super) fn select(&self, now: OffsetDateTime) -> Result<(usize, &CertificateBundle)> {
        self.bundles
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                b.not_valid_before <= now.unix_timestamp()
                    && now.unix_timestamp() < b.not_valid_after
            })
            .max_by_key(|(_, b)| (b.not_valid_after, b.not_valid_before))
            .context("no valid local Cast bundle window")
    }

    pub(super) fn at(&self, index: usize) -> &CertificateBundle {
        &self.bundles[index]
    }
}
