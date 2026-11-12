//! Loopback TLS clients: no LAN announcement, external media fetch or real playback.

use super::*;
use crate::proto::cast_channel::{
    AuthChallenge, AuthResponse, CastMessage, DeviceAuthMessage, HashAlgorithm, PayloadType,
    SignatureAlgorithm,
};
use prost::Message;
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug)]
struct TestCertificate(Option<Vec<u8>>);
impl ServerCertVerifier for TestCertificate {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if self
            .0
            .as_ref()
            .is_some_and(|expected| end.as_ref() != expected)
        {
            return Err(rustls::Error::General(
                "unexpected loopback certificate".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

type Client = tokio_rustls::client::TlsStream<tokio::net::TcpStream>;

async fn client() -> (Client, Arc<Bridge>, tokio::task::JoinHandle<()>) {
    client_with_identity(crypto::Identity::generate().unwrap()).await
}

async fn client_with_identity(
    identity: crypto::Identity,
) -> (Client, Arc<Bridge>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let expected = identity.tls_cert_der().to_vec();
    let bridge = Arc::new(Bridge::default());
    let shared = server::Shared {
        state: Arc::new(tokio::sync::Mutex::new(state::AppState::default())),
        identity: Arc::new(identity),
        player: bridge.spawn(),
        connection_filter: None,
        discovery: Some(receiver::DiscoveryInfo {
            id: uuid::Uuid::parse_str("e8d8571a824a45df88929e1c220f602d").unwrap(),
            name: "Synthetic Echo".into(),
            model: "OpenChromecast Android".into(),
            capabilities: 5,
        }),
        youtube: None,
    };
    let task = tokio::spawn(async move {
        let _ = server::run(listener, shared).await;
    });
    let client = connect_loopback(address, Some(expected)).await.unwrap();
    (client, bridge, task)
}

async fn connect_loopback(
    address: std::net::SocketAddr,
    expected: Option<Vec<u8>>,
) -> anyhow::Result<Client> {
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(TestCertificate(expected)))
    .with_no_client_auth();
    let stream = tokio::net::TcpStream::connect(address).await?;
    let client = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from("localhost").unwrap(), stream)
        .await?;
    Ok(client)
}

async fn auth_response(client: &mut Client, nonce: Vec<u8>) -> AuthResponse {
    let auth = DeviceAuthMessage {
        challenge: Some(AuthChallenge {
            sender_nonce: Some(nonce),
            signature_algorithm: Some(SignatureAlgorithm::RsassaPkcs1v15 as i32),
            hash_algorithm: Some(HashAlgorithm::Sha256 as i32),
        }),
        ..Default::default()
    };
    let message = CastMessage {
        protocol_version: 0,
        source_id: "sender-rotation-test".into(),
        destination_id: "receiver-0".into(),
        namespace: server::NS_DEVICE_AUTH.into(),
        payload_type: PayloadType::Binary as i32,
        payload_binary: Some(auth.encode_to_vec()),
        ..Default::default()
    }
    .encode_to_vec();
    client.write_u32(message.len() as u32).await.unwrap();
    client.write_all(&message).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let length = client.read_u32().await.unwrap() as usize;
            assert!(length < 1024 * 1024);
            let mut bytes = vec![0; length];
            client.read_exact(&mut bytes).await.unwrap();
            let frame = CastMessage::decode(bytes.as_slice()).unwrap();
            if frame.namespace == server::NS_DEVICE_AUTH {
                return DeviceAuthMessage::decode(frame.payload_binary.unwrap().as_slice())
                    .unwrap()
                    .response
                    .unwrap();
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn connection_filter_rejects_before_tls_and_applies_to_each_new_connection() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let listener = tokio::net::TcpListener::bind(("::1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let identity = crypto::Identity::generate().unwrap();
    let expected = identity.tls_cert_der().to_vec();
    let allowed = Arc::new(AtomicBool::new(false));
    let gate = allowed.clone();
    let bridge = Arc::new(Bridge::default());
    let shared = server::Shared {
        state: Arc::new(tokio::sync::Mutex::new(state::AppState::default())),
        identity: Arc::new(identity),
        player: bridge.spawn(),
        connection_filter: Some(Arc::new(move |_| gate.load(Ordering::SeqCst))),
        discovery: None,
        youtube: None,
    };
    let task = tokio::spawn(async move {
        let _ = server::run(listener, shared).await;
    });
    let mut denied = tokio::net::TcpStream::connect(address).await.unwrap();
    let mut byte = [0];
    let read = tokio::time::timeout(std::time::Duration::from_secs(2), denied.read(&mut byte))
        .await
        .unwrap();
    assert!(
        matches!(read, Ok(0) | Err(_)),
        "TLS bytes sent before network policy"
    );
    allowed.store(true, Ordering::SeqCst);
    let mut accepted = connect_loopback(address, Some(expected)).await.unwrap();
    assert!(
        auth_response(&mut accepted, vec![0x42; 32])
            .await
            .sender_nonce
            .is_some()
    );
    assert!(!bridge.poll().iter().any(|event| event["type"] == "load"));
    task.abort();
}

#[tokio::test]
async fn tls_rotation_preserves_existing_connection_auth_and_active_media() {
    use std::sync::atomic::{AtomicI64, Ordering};
    let (identity, leaf, chain) = synthetic_device_identity(2);
    let identity = Arc::new(identity);
    let old_certificate = identity.tls_cert_der().to_vec();
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let clock = Arc::new(AtomicI64::new(now));
    let server_clock = clock.clone();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let bridge = Arc::new(Bridge::default());
    let shared = server::Shared {
        state: Arc::new(tokio::sync::Mutex::new(state::AppState::default())),
        identity: identity.clone(),
        player: bridge.spawn(),
        connection_filter: None,
        discovery: None,
        youtube: None,
    };
    let task = tokio::spawn(async move {
        server::run_with_clock(
            listener,
            shared,
            Arc::new(move || {
                time::OffsetDateTime::from_unix_timestamp(server_clock.load(Ordering::Acquire))
                    .unwrap()
            }),
            std::time::Duration::from_millis(20),
        )
        .await
        .unwrap();
    });
    let mut old = connect_loopback(address, Some(old_certificate.clone()))
        .await
        .unwrap();
    let launched = request(
        &mut old,
        server::NS_RECEIVER,
        "receiver-0",
        json!({"type":"LAUNCH","appId":"CC1AD845","requestId":1}),
    )
    .await;
    let transport = launched["status"]["applications"][0]["transportId"]
        .as_str()
        .unwrap()
        .to_owned();
    request(
        &mut old,
        server::NS_MEDIA,
        &transport,
        json!({"type":"LOAD","requestId":2,
        "media":{"contentId":"https://example.test/rotation.wav","contentType":"audio/wav"}}),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let events = bridge.poll();
    let generation = events.iter().find(|e| e["type"] == "load").unwrap()["generation"]
        .as_u64()
        .unwrap();
    bridge.update(
        generation,
        PlayerSnapshot {
            state: PlayerState::Playing,
            position: 17.0,
            duration: 60.0,
            ..Default::default()
        },
    );
    // Only the injected certificate clock moves. No system clock or real media is involved.
    clock.store(now + 49 * 3600, Ordering::Release);
    let mut fresh = tokio::time::timeout(std::time::Duration::from_secs(45), async {
        loop {
            if let Ok(client) = connect_loopback(address, None).await {
                let peer = client.get_ref().1.peer_certificates().unwrap()[0].as_ref();
                if peer != old_certificate {
                    break client;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let fresh_certificate = fresh.get_ref().1.peer_certificates().unwrap()[0]
        .as_ref()
        .to_vec();
    for (client, certificate, nonce) in [
        (&mut old, old_certificate, vec![0x21; 32]),
        (&mut fresh, fresh_certificate, vec![0x73; 32]),
    ] {
        let response = auth_response(client, nonce.clone()).await;
        let mut input = nonce.clone();
        input.extend_from_slice(&certificate);
        assert_eq!(response.client_auth_certificate, leaf);
        assert_eq!(response.intermediate_certificate, chain);
        assert_eq!(response.sender_nonce, Some(nonce));
        assert_eq!(
            response.signature,
            identity.sign(&input).unwrap(),
            "wrong TLS epoch in auth response"
        );
    }
    let status = request(
        &mut old,
        server::NS_MEDIA,
        &transport,
        json!({"type":"GET_STATUS","requestId":3}),
    )
    .await;
    assert_eq!(status["status"][0]["playerState"], "PLAYING");
    assert_eq!(status["status"][0]["currentTime"], 17.0);
    assert!(!bridge.poll().iter().any(|e| e["type"] == "stop"));
    drop(fresh);
    drop(old);
    task.abort();
}

fn synthetic_device_identity(count: usize) -> (crypto::Identity, Vec<u8>, Vec<Vec<u8>>) {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};

    let ca_params = |name: String| {
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, name);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params
    };
    let root_key = KeyPair::generate().unwrap();
    let root = ca_params("Cast test root".into())
        .self_signed(&root_key)
        .unwrap();
    let mut issuers = vec![(root, root_key)];
    for index in 0..count {
        let key = KeyPair::generate().unwrap();
        let (issuer, issuer_key) = issuers.last().unwrap();
        let certificate = ca_params(format!("Cast test intermediate {index}"))
            .signed_by(&key, issuer, issuer_key)
            .unwrap();
        issuers.push((certificate, key));
    }
    let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
    let key_pem = key.to_pkcs8_pem(LineEnding::LF).unwrap();
    let key_pair = KeyPair::from_pem(&key_pem).unwrap();
    let (issuer, issuer_key) = issuers.last().unwrap();
    let leaf = CertificateParams::new(vec!["cast-test.invalid".into()])
        .unwrap()
        .signed_by(&key_pair, issuer, issuer_key)
        .unwrap();
    let chain: Vec<_> = issuers.iter().skip(1).rev().map(|(cert, _)| cert).collect();
    let pem = leaf.pem() + &chain.iter().map(|cert| cert.pem()).collect::<String>();
    let expected_chain = chain.iter().map(|cert| cert.der().to_vec()).collect();

    // Only newly generated test credentials enter this private temporary directory.
    let directory = std::env::temp_dir().join(format!("cast-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let cert_path = directory.join("certificate.pem");
    let key_path = directory.join("key.pem");
    std::fs::write(&cert_path, pem).unwrap();
    std::fs::write(&key_path, key_pem.as_bytes()).unwrap();
    let identity = crypto::Identity::load_real_device(&cert_path, &key_path);
    std::fs::remove_file(cert_path).unwrap();
    std::fs::remove_file(key_path).unwrap();
    std::fs::remove_dir(directory).unwrap();
    (identity.unwrap(), leaf.der().to_vec(), expected_chain)
}

#[cfg(feature = "legacy-bundle")]
#[tokio::test]
async fn legacy_rotation_on_accept_keeps_existing_tls_auth_and_media_epoch() {
    use std::sync::atomic::{AtomicI64, Ordering};
    let now = time::OffsetDateTime::now_utc();
    let json = crypto::legacy_tests::fixture(now);
    let identity = Arc::new(crypto::Identity::from_legacy_bundle(&json).unwrap());
    let expected_next = identity
        .renew_tls_at(now + time::Duration::seconds(60))
        .unwrap();
    let old_der = identity.tls_cert_der().to_vec();
    let clock = Arc::new(AtomicI64::new(now.unix_timestamp()));
    let server_clock = clock.clone();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let bridge = Arc::new(Bridge::default());
    let shared = server::Shared {
        state: Arc::new(tokio::sync::Mutex::new(state::AppState::default())),
        identity: identity.clone(),
        player: bridge.spawn(),
        connection_filter: None,
        discovery: None,
        youtube: None,
    };
    let task = tokio::spawn(async move {
        server::run_with_clock(
            listener,
            shared,
            Arc::new(move || {
                time::OffsetDateTime::from_unix_timestamp(server_clock.load(Ordering::Acquire))
                    .unwrap()
            }),
            std::time::Duration::from_secs(3600),
        )
        .await
        .unwrap();
    });
    let mut old = connect_loopback(address, Some(old_der)).await.unwrap();
    let launched = request(
        &mut old,
        server::NS_RECEIVER,
        "receiver-0",
        json!({"type":"LAUNCH","appId":"CC1AD845","requestId":1}),
    )
    .await;
    let transport = launched["status"]["applications"][0]["transportId"]
        .as_str()
        .unwrap()
        .to_owned();
    request(
        &mut old,
        server::NS_MEDIA,
        &transport,
        json!({"type":"LOAD","requestId":2,
        "media":{"contentId":"https://example.test/legacy.wav","contentType":"audio/wav"}}),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let events = bridge.poll();
    let generation = events.iter().find(|e| e["type"] == "load").unwrap()["generation"]
        .as_u64()
        .unwrap();
    bridge.update(
        generation,
        PlayerSnapshot {
            state: PlayerState::Playing,
            position: 17.0,
            duration: 60.0,
            ..Default::default()
        },
    );
    clock.store(now.unix_timestamp() + 60, Ordering::Release);
    // No timer tick: the first new connection must rotate an expired window immediately.
    let mut fresh = connect_loopback(address, Some(expected_next.tls_cert_der().to_vec()))
        .await
        .unwrap();
    let challenge = AuthChallenge {
        sender_nonce: Some(vec![0x21; 32]),
        signature_algorithm: Some(1),
        hash_algorithm: Some(1),
    };
    for (client, expected) in [(&mut old, &*identity), (&mut fresh, &expected_next)] {
        let response = auth_response(client, challenge.sender_nonce.clone().unwrap()).await;
        assert_eq!(response, expected.device_auth_response(&challenge).unwrap());
        assert_eq!(response.sender_nonce, None);
    }
    let status = request(
        &mut old,
        server::NS_MEDIA,
        &transport,
        json!({"type":"GET_STATUS","requestId":3}),
    )
    .await;
    assert_eq!(status["status"][0]["playerState"], "PLAYING");
    assert_eq!(status["status"][0]["currentTime"], 17.0);
    assert!(!bridge.poll().iter().any(|event| event["type"] == "stop"));
    drop(fresh);
    drop(old);
    task.abort();
}

#[tokio::test]
async fn device_auth_keeps_complete_certificate_chain_and_nonce_binding() {
    for count in [0, 1, 2] {
        let (identity, expected_leaf, expected_chain) = synthetic_device_identity(count);
        let nonce = vec![0x81; 32];
        let mut signed = nonce.clone();
        signed.extend_from_slice(identity.tls_cert_der());
        let expected_signature = identity.sign(&signed).unwrap();
        let (mut stream, _, task) = client_with_identity(identity).await;
        let challenge = DeviceAuthMessage {
            challenge: Some(AuthChallenge {
                sender_nonce: Some(nonce.clone()),
                signature_algorithm: Some(SignatureAlgorithm::RsassaPkcs1v15 as i32),
                hash_algorithm: Some(HashAlgorithm::Sha256 as i32),
            }),
            ..Default::default()
        };
        let message = CastMessage {
            protocol_version: 0,
            source_id: "sender-test".into(),
            destination_id: "receiver-0".into(),
            namespace: server::NS_DEVICE_AUTH.into(),
            payload_type: PayloadType::Binary as i32,
            payload_binary: Some(challenge.encode_to_vec()),
            ..Default::default()
        }
        .encode_to_vec();
        stream.write_u32(message.len() as u32).await.unwrap();
        stream.write_all(&message).await.unwrap();
        let response = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let size = stream.read_u32().await.unwrap() as usize;
                assert!(size <= 1024 * 1024);
                let mut bytes = vec![0; size];
                stream.read_exact(&mut bytes).await.unwrap();
                let envelope = CastMessage::decode(bytes.as_slice()).unwrap();
                if envelope.namespace == server::NS_DEVICE_AUTH {
                    assert_eq!(envelope.source_id, "receiver-0");
                    assert_eq!(envelope.destination_id, "sender-test");
                    break DeviceAuthMessage::decode(envelope.payload_binary.unwrap().as_slice())
                        .unwrap()
                        .response
                        .unwrap();
                }
            }
        })
        .await
        .unwrap();
        task.abort();
        assert_eq!(
            response.intermediate_certificate.len(),
            count,
            "chain was truncated"
        );
        assert!(
            response.client_auth_certificate == expected_leaf,
            "leaf changed"
        );
        assert!(
            response.intermediate_certificate == expected_chain,
            "chain order changed"
        );
        assert_eq!(response.sender_nonce, Some(nonce));
        assert!(
            response.signature == expected_signature,
            "nonce/TLS binding changed"
        );
        assert_eq!(response.hash_algorithm, Some(HashAlgorithm::Sha256 as i32));
    }
}

async fn request(
    client: &mut Client,
    namespace: &str,
    destination: &str,
    payload: serde_json::Value,
) -> serde_json::Value {
    let request_id = payload["requestId"].clone();
    let message = CastMessage {
        protocol_version: 0,
        source_id: "sender-test".into(),
        destination_id: destination.into(),
        namespace: namespace.into(),
        payload_type: PayloadType::String as i32,
        payload_utf8: Some(payload.to_string()),
        ..Default::default()
    }
    .encode_to_vec();
    client.write_u32(message.len() as u32).await.unwrap();
    client.write_all(&message).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let size = client.read_u32().await.unwrap() as usize;
            assert!(size <= 1024 * 1024);
            let mut data = vec![0; size];
            client.read_exact(&mut data).await.unwrap();
            let response = CastMessage::decode(data.as_slice()).unwrap();
            let value: serde_json::Value =
                serde_json::from_str(response.payload_utf8.as_deref().unwrap()).unwrap();
            if value["requestId"] == request_id {
                return value;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn tls_receiver_status() {
    let (mut stream, _, task) = client().await;
    let status = request(
        &mut stream,
        server::NS_RECEIVER,
        "receiver-0",
        json!({"type":"GET_STATUS","requestId":1}),
    )
    .await;
    assert_eq!(status["type"], "RECEIVER_STATUS");
    assert!(
        status["status"]["applications"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    task.abort();
}

#[tokio::test]
async fn canonical_app_availability_is_correlated_and_does_not_launch() {
    let (mut stream, bridge, task) = client().await;
    let result = request(
        &mut stream,
        server::NS_RECEIVER,
        "receiver-0",
        json!({"type":"GET_APP_AVAILABILITY","requestId":71,"appId":["CC1AD845","UNKNOWN_TEST"]}),
    )
    .await;
    assert_eq!(result["type"], "GET_APP_AVAILABILITY");
    assert_eq!(result["availability"]["CC1AD845"], "APP_AVAILABLE");
    assert_eq!(result["availability"]["UNKNOWN_TEST"], "APP_UNAVAILABLE");
    let status = request(
        &mut stream,
        server::NS_RECEIVER,
        "receiver-0",
        json!({"type":"GET_STATUS","requestId":72}),
    )
    .await;
    assert!(
        status["status"]["applications"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(bridge.poll().iter().all(|e| e["type"] != "load"));
    task.abort();
}

#[tokio::test]
async fn discovery_namespace_returns_truthful_compact_identity_and_video_audio_capabilities() {
    let (mut stream, bridge, task) = client().await;
    let result = request(
        &mut stream,
        "urn:x-cast:com.google.cast.receiver.discovery",
        "receiver-0",
        json!({"type":"GET_DEVICE_INFO","requestId":73}),
    )
    .await;
    assert_eq!(result["type"], "DEVICE_INFO");
    assert_eq!(result["deviceId"], "e8d8571a824a45df88929e1c220f602d");
    assert_eq!(result["friendlyName"], "Synthetic Echo");
    assert_eq!(result["deviceModel"], "OpenChromecast Android");
    assert_eq!(result["deviceCapabilities"], 5);
    assert!(bridge.poll().iter().all(|e| e["type"] != "load"));
    task.abort();
}

#[tokio::test]
async fn media_load_reaches_host_and_real_status_returns_to_sender() {
    let (mut stream, bridge, task) = client().await;
    let launched = request(
        &mut stream,
        server::NS_RECEIVER,
        "receiver-0",
        json!({"type":"LAUNCH","appId":"CC1AD845","requestId":1}),
    )
    .await;
    let transport = launched["status"]["applications"][0]["transportId"]
        .as_str()
        .unwrap();
    request(&mut stream,server::NS_MEDIA,transport,json!({"type":"LOAD","requestId":2,"media":{"contentId":"https://example.test/test.mp3","contentType":"audio/mpeg","streamType":"BUFFERED"},"autoplay":true})).await;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let events = bridge.poll();
    let load = events.iter().find(|e| e["type"] == "load").unwrap();
    assert_eq!(load["video"], false);
    let generation = load["generation"].as_u64().unwrap();
    bridge.update(
        generation,
        PlayerSnapshot {
            state: PlayerState::Playing,
            position: 17.0,
            duration: 60.0,
            ..Default::default()
        },
    );
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let status = request(
        &mut stream,
        server::NS_MEDIA,
        transport,
        json!({"type":"GET_STATUS","requestId":3}),
    )
    .await;
    assert_eq!(status["status"][0]["playerState"], "PLAYING");
    assert_eq!(status["status"][0]["currentTime"], 17.0);
    request(
        &mut stream,
        server::NS_MEDIA,
        transport,
        json!({"type":"PAUSE","requestId":4}),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(bridge.poll().iter().any(|e| e["type"] == "pause"));
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let stopped = request(
        &mut stream,
        server::NS_MEDIA,
        transport,
        json!({"type":"STOP","requestId":5}),
    )
    .await;
    assert_eq!(stopped["status"][0]["playerState"], "IDLE");
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        bridge.poll().iter().any(|e| e["type"] == "stop"),
        "embedded stop must not wait for the desktop VLC debounce"
    );
    let terminal = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let length = stream.read_u32().await.unwrap() as usize;
            assert!(length < 1024 * 1024);
            let mut frame = vec![0; length];
            stream.read_exact(&mut frame).await.unwrap();
            let envelope = CastMessage::decode(frame.as_slice()).unwrap();
            if envelope.namespace != server::NS_MEDIA {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_str(envelope.payload_utf8.as_deref().unwrap()).unwrap();
            if value.get("requestId").is_none() && value["status"][0]["playerState"] == "IDLE" {
                break value;
            }
        }
    })
    .await
    .expect("host terminal state must reach subscribed senders");
    assert_eq!(terminal["status"][0]["idleReason"], "CANCELLED");
    task.abort();
}
