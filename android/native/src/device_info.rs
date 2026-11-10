use std::sync::Arc;

use axum::{Json, Router, extract::State, http::header, routing::get};
use serde_json::{Value, json};
use uuid::Uuid;

pub const HTTP_PORT: u16 = 8008;

#[derive(Clone)]
pub struct DeviceInfo {
    id: Uuid,
    name: String,
}

impl DeviceInfo {
    pub fn new(id: &str, name: String) -> anyhow::Result<Self> {
        let id = Uuid::parse_str(id)?;
        anyhow::ensure!(!id.is_nil(), "invalid discovery identity");
        anyhow::ensure!(
            !name.trim().is_empty() && name.len() <= 63 && !name.chars().any(char::is_control),
            "invalid discovery name"
        );
        Ok(Self { id, name })
    }

    fn document(&self) -> Value {
        json!({
            "name": self.name,
            "ssdp_udn": self.id.to_string(),
            "device_info": {
                "name": self.name,
                "ssdp_udn": self.id.to_string(),
                "model_name": "OpenChromecast Android",
                "manufacturer": "OpenChromecast",
                "capabilities": {"display_supported": true, "multizone_supported": false}
            },
            "multizone": {"groups": [], "dynamic_groups": []}
        })
    }
}

async fn eureka_info(State(info): State<Arc<DeviceInfo>>) -> impl axum::response::IntoResponse {
    ([(header::CACHE_CONTROL, "no-store")], Json(info.document()))
}

pub fn router(info: DeviceInfo) -> Router {
    Router::new()
        .route("/setup/eureka_info", get(eureka_info))
        .with_state(Arc::new(info))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    fn fixture() -> DeviceInfo {
        DeviceInfo::new(
            "e8d8571a824a45df88929e1c220f602d",
            "Receiver \"A&B\"".into(),
        )
        .unwrap()
    }

    #[test]
    fn validates_stable_identity_and_name() {
        assert!(DeviceInfo::new("invalid", "Receiver".into()).is_err());
        assert!(DeviceInfo::new(&Uuid::nil().to_string(), "Receiver".into()).is_err());
        for name in [" ".into(), "bad\nname".into(), "x".repeat(64)] {
            assert!(DeviceInfo::new(&fixture().id.to_string(), name).is_err());
        }
    }

    #[tokio::test]
    async fn returns_consistent_truthful_discovery_metadata() {
        let info = fixture();
        let expected_id = info.id;
        let expected_name = info.name.clone();
        let response = router(info)
            .oneshot(
                Request::builder()
                    .uri("/setup/eureka_info?params=device_info,name")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let document: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(document["name"], expected_name);
        assert_eq!(
            Uuid::parse_str(document["device_info"]["ssdp_udn"].as_str().unwrap()).unwrap(),
            expected_id
        );
        assert_eq!(document["device_info"]["manufacturer"], "OpenChromecast");
        assert_eq!(
            document["device_info"]["capabilities"]["multizone_supported"],
            false
        );
        assert_eq!(document["multizone"]["groups"], json!([]));
    }

    #[tokio::test]
    async fn refuses_writes_and_does_not_advertise_a_youtube_receiver() {
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            let response = router(fixture())
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri("/setup/eureka_info")
                        .body(Body::from("PRIVATE_SENTINEL"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        }
        for path in ["/apps/YouTube", "/setup/reboot", "/"] {
            let response = router(fixture())
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }
}
