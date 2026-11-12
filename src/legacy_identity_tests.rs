use super::*;
use crate::proto::cast_channel::AuthChallenge;
use base64::{Engine, engine::general_purpose::STANDARD};
use rsa::signature::Verifier;
use serde_json::json;
use std::sync::OnceLock;

pub(crate) fn fixture(now: OffsetDateTime) -> String {
    static KEY: OnceLock<String> = OnceLock::new();
    let pem = KEY.get_or_init(|| {
        RsaPrivateKey::new(&mut OsRng, 2048)
            .unwrap()
            .to_pkcs8_pem(LineEnding::LF)
            .unwrap()
            .to_string()
    });
    let key = RsaPrivateKey::from_pkcs8_pem(pem).unwrap();
    let pair = rcgen::KeyPair::from_pem(pem).unwrap();
    let certificate = |start, end| {
        let mut params = rcgen::CertificateParams::new(Vec::new()).unwrap();
        params.not_before = start;
        params.not_after = end;
        params.self_signed(&pair).unwrap()
    };
    let auth = certificate(now - Duration::days(1), now + Duration::days(30));
    let windows: Vec<_> = [(now - Duration::hours(1), now + Duration::seconds(60)),
        (now + Duration::seconds(60), now + Duration::hours(2))].into_iter().map(|(start, end)| {
        let tls = certificate(start, end);
        let sha256 = SigningKey::<Sha256>::new(key.clone()).sign(tls.der()).to_vec();
        let sha1 = SigningKey::<sha1::Sha1>::new(key.clone()).sign(tls.der()).to_vec();
        json!({"pu":tls.pem(), "pr":pem, "sig_sha256":STANDARD.encode(sha256), "sig_sha1":STANDARD.encode(sha1)})
    }).collect();
    json!({"cpu":auth.pem(), "ica":auth.pem(), "certs":windows}).to_string()
}

#[test]
fn legacy_response_binds_tls_only_and_cannot_forge_a_fresh_nonce() {
    let identity = Identity::from_legacy_bundle(&fixture(OffsetDateTime::now_utc())).unwrap();
    assert!(identity.sign(b"fresh nonce").is_err());
    for hash in [0, 1] {
        let challenge = AuthChallenge {
            sender_nonce: Some(vec![0x71; 32]),
            signature_algorithm: Some(1),
            hash_algorithm: Some(hash),
        };
        let response = identity.device_auth_response(&challenge).unwrap();
        assert_eq!(response.sender_nonce, None);
        assert_eq!(response.hash_algorithm, Some(hash));
        let mut nonce_tls = challenge.sender_nonce.unwrap();
        nonce_tls.extend_from_slice(identity.tls_cert_der());
        let pem = fixture(OffsetDateTime::now_utc());
        let material: serde_json::Value = serde_json::from_str(&pem).unwrap();
        let key =
            RsaPrivateKey::from_pkcs8_pem(material["certs"][0]["pr"].as_str().unwrap()).unwrap();
        let signature = rsa::pkcs1v15::Signature::try_from(response.signature.as_slice()).unwrap();
        if hash == 1 {
            let verifier = rsa::pkcs1v15::VerifyingKey::<Sha256>::new(key.to_public_key());
            verifier
                .verify(identity.tls_cert_der(), &signature)
                .unwrap();
            assert!(verifier.verify(&nonce_tls, &signature).is_err());
        } else {
            rsa::pkcs1v15::VerifyingKey::<sha1::Sha1>::new(key.to_public_key())
                .verify(identity.tls_cert_der(), &signature)
                .unwrap();
        }
    }
    for challenge in [
        AuthChallenge {
            signature_algorithm: Some(2),
            ..Default::default()
        },
        AuthChallenge {
            signature_algorithm: Some(1),
            hash_algorithm: Some(99),
            ..Default::default()
        },
    ] {
        assert!(identity.device_auth_response(&challenge).is_err());
    }
}

#[test]
fn local_rotation_has_exclusive_boundaries_and_no_self_signed_fallback() {
    let now = OffsetDateTime::now_utc();
    let identity = Identity::from_legacy_bundle(&fixture(now)).unwrap();
    assert!(!identity.tls_renewal_due_at(now + Duration::seconds(59)));
    assert!(identity.tls_renewal_due_at(now + Duration::seconds(60)));
    let next = identity.renew_tls_at(now + Duration::seconds(60)).unwrap();
    assert_ne!(identity.tls_cert_der(), next.tls_cert_der());
    assert_eq!(identity.auth_cert_der(), next.auth_cert_der());
    assert!(!identity.tls_valid_at(now + Duration::seconds(60)));
    assert!(next.tls_valid_at(now + Duration::seconds(60)));
    assert!(identity.renew_tls_at(now + Duration::days(1)).is_err());
    assert!(identity.renew_tls_at(now - Duration::days(1)).is_err());
    assert_eq!(
        next.renew_tls_at(now).unwrap().tls_cert_der(),
        identity.tls_cert_der()
    );
}

#[test]
fn corrupt_signatures_keys_and_windows_fail_closed() {
    let original: serde_json::Value =
        serde_json::from_str(&fixture(OffsetDateTime::now_utc())).unwrap();
    for field in ["pu", "pr", "sig_sha1", "sig_sha256"] {
        let mut value = original.clone();
        value["certs"][0][field] = json!("corrupt");
        assert!(Identity::from_legacy_bundle(&value.to_string()).is_err());
    }
    let mut value = original;
    value["certs"] = json!([]);
    assert!(Identity::from_legacy_bundle(&value.to_string()).is_err());
    assert!(Identity::from_legacy_bundle("not JSON").is_err());
}
