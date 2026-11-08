//! Loopback TLS tests: no LAN announcement, external media fetch or real playback.

use super::*;
use crate::proto::cast_channel::{CastMessage, PayloadType};
use prost::Message;
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug)]
struct TestCertificate(Vec<u8>);
impl ServerCertVerifier for TestCertificate {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end.as_ref() != self.0 {
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
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let identity = crypto::Identity::generate().unwrap();
    let verifier = TestCertificate(identity.tls_cert_der().to_vec());
    let bridge = Arc::new(Bridge::default());
    let shared = server::Shared {
        state: Arc::new(tokio::sync::Mutex::new(state::AppState::default())),
        identity: Arc::new(identity),
        player: bridge.spawn(),
    };
    let task = tokio::spawn(async move {
        let _ = server::run(listener, shared).await;
    });
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(verifier))
    .with_no_client_auth();
    let stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let client = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from("localhost").unwrap(), stream)
        .await
        .unwrap();
    (client, bridge, task)
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
    task.abort();
}
