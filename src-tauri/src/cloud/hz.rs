use std::time::Duration;

use reqwest::Client;

use crate::cloud::{map_http_error, CloudError, CloudProvider, ProvisionParams, VpsInstance};

// ---------------------------------------------------------------------------
// Production code
// ---------------------------------------------------------------------------

const HZ_API_BASE: &str = "https://api.hetzner.cloud/v1";

/// Hetzner Cloud API provider.
///
/// # Auth
/// Uses `Authorization: Bearer {token}` on every request. The token is never
/// stored on the struct — it is passed as a `&str` parameter to each method.
pub struct HetznerProvider {
    client: Client,
    base_url: String,
}

impl HetznerProvider {
    /// Create a new provider targeting the production Hetzner Cloud API.
    pub fn new() -> Result<Self, CloudError> {
        Self::with_base_url(HZ_API_BASE)
    }

    /// Create a provider with a custom base URL (used for testing with wiremock).
    pub(crate) fn with_base_url(base_url: &str) -> Result<Self, CloudError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(CloudError::Http)?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    // -- convenience request helpers ---------------------------------------

    async fn get(&self, path: &str, token: &str) -> Result<reqwest::Response, CloudError> {
        self.client
            .get(format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .send()
            .await
            .map_err(CloudError::Http)
    }

    async fn post(
        &self,
        path: &str,
        token: &str,
        body: serde_json::Value,
    ) -> Result<reqwest::Response, CloudError> {
        self.client
            .post(format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(CloudError::Http)
    }

    async fn delete(&self, path: &str, token: &str) -> Result<reqwest::Response, CloudError> {
        self.client
            .delete(format!("{}{}", self.base_url, path))
            .bearer_auth(token)
            .send()
            .await
            .map_err(CloudError::Http)
    }
}

impl CloudProvider for HetznerProvider {
    async fn validate_token(&self, token: &str) -> Result<bool, CloudError> {
        let response = self.get("/v1/datacenters", token).await?;
        if response.status().is_success() {
            Ok(true)
        } else {
            Err(map_http_error(response).await)        }
    }

    async fn create_vps(&self, params: &ProvisionParams, token: &str) -> Result<VpsInstance, CloudError> {
        let body = serde_json::json!({
            "name": format!("vpn-{}", params.region),
            "server_type": params.size,
            "image": params.image,
            "location": params.region,
        });
        let response = self.post("/v1/servers", token, body).await?;
        let status = response.status();
        if status.is_success() || status.as_u16() == 201 {
            let data: serde_json::Value = response.json().await.map_err(CloudError::Http)?;
            Ok(parse_server(&data["server"], "hetzner"))
        } else {
            Err(map_http_error(response).await)        }
    }

    async fn destroy_vps(&self, instance_id: &str, token: &str) -> Result<(), CloudError> {
        let response = self.delete(&format!("/v1/servers/{}", instance_id), token).await?;
        if response.status().is_success() || response.status().as_u16() == 204 {
            Ok(())
        } else {
            Err(map_http_error(response).await)        }
    }

    async fn list_vpss(&self, token: &str) -> Result<Vec<VpsInstance>, CloudError> {
        let response = self.get("/v1/servers", token).await?;
        let status = response.status();
        if status.is_success() {
            let data: serde_json::Value = response.json().await.map_err(CloudError::Http)?;
            let servers = data["servers"]
                .as_array()
                .map(|arr| arr.iter().map(|s| parse_server(s, "hetzner")).collect())
                .unwrap_or_default();
            Ok(servers)
        } else {
            Err(map_http_error(response).await)        }
    }
}

// ---------------------------------------------------------------------------
// Response parsing helper
// ---------------------------------------------------------------------------

/// Extract a `VpsInstance` from a Hetzner Server JSON object.
fn parse_server(value: &serde_json::Value, provider: &str) -> VpsInstance {
    let ip = value["public_net"]["ipv4"]["ip"]
        .as_str()
        .or_else(|| {
            value["public_net"]["ipv6"]["ip"]
                .as_str()
        })
        .unwrap_or("")
        .to_string();

    VpsInstance {
        id: value["id"].to_string(),
        provider: provider.to_string(),
        region: value["datacenter"]["location"]["name"]
            .as_str()
            .or_else(|| value["location"].as_str())
            .unwrap_or("")
            .to_string(),
        ip,
        status: value["status"].as_str().unwrap_or("").to_string(),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Helper: start a wiremock server and attach a `HetznerProvider`.
    async fn setup_hz() -> (MockServer, HetznerProvider) {
        let mock_server = MockServer::start().await;
        let base_url = mock_server.uri();
        let provider = HetznerProvider::with_base_url(&base_url).unwrap();
        (mock_server, provider)
    }

    // -----------------------------------------------------------------------
    // 3.3.a — Token validation (valid / invalid)
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn validate_token_valid() {
        let (mock_server, provider) = setup_hz().await;

        Mock::given(method("GET"))
            .and(path("/v1/datacenters"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let result = provider.validate_token("hz-test-token").await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn validate_token_invalid() {
        let (mock_server, provider) = setup_hz().await;

        Mock::given(method("GET"))
            .and(path("/v1/datacenters"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let result = provider.validate_token("bad-token").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CloudError::Auth(_) => { /* expected */ }
            other => panic!("expected Auth error, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // 3.3.b — Create server
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn create_server_success() {
        let (mock_server, provider) = setup_hz().await;

        let body = serde_json::json!({
            "server": {
                "id": 42,
                "name": "vpn-fsn1",
                "status": "running",
                "public_net": {
                    "ipv4": { "ip": "1.2.3.4" },
                    "ipv6": { "ip": "2001:db8::1" }
                },
                "datacenter": {
                    "location": { "name": "fsn1" }
                }
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/servers"))
            .respond_with(ResponseTemplate::new(201).set_body_json(body))
            .mount(&mock_server)
            .await;

        let params = ProvisionParams {
            provider: "hetzner".into(),
            region: "fsn1".into(),
            size: "cx22".into(),
            image: "ubuntu-24-04".into(),
        };

        let result = provider.create_vps(&params, "hz-token").await;
        assert!(result.is_ok(), "expected Ok, got {:?}", result);

        let instance = result.unwrap();
        assert_eq!(instance.provider, "hetzner");
        assert_eq!(instance.ip, "1.2.3.4");
        assert_eq!(instance.status, "running");
        assert_eq!(instance.region, "fsn1");
    }

    // -----------------------------------------------------------------------
    // 3.3.c — Destroy missing server
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn destroy_missing_server() {
        let (mock_server, provider) = setup_hz().await;

        Mock::given(method("DELETE"))
            .and(path("/v1/servers/99999"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let result = provider.destroy_vps("99999", "hz-token").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CloudError::Provider(_) => { /* expected */ }
            other => panic!("expected Provider error, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // 3.3.d — List servers
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn list_servers_success() {
        let (mock_server, provider) = setup_hz().await;

        let body = serde_json::json!({
            "servers": [
                {
                    "id": 101,
                    "name": "server-1",
                    "status": "running",
                    "public_net": {
                        "ipv4": { "ip": "10.0.0.1" },
                        "ipv6": { "ip": "2001:db8::1" }
                    },
                    "datacenter": {
                        "location": { "name": "fsn1" }
                    }
                },
                {
                    "id": 102,
                    "name": "server-2",
                    "status": "off",
                    "public_net": {
                        "ipv4": { "ip": "10.0.0.2" },
                        "ipv6": { "ip": "2001:db8::2" }
                    },
                    "datacenter": {
                        "location": { "name": "nbg1" }
                    }
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/v1/servers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&mock_server)
            .await;

        let result = provider.list_vpss("hz-token").await;
        assert!(result.is_ok(), "expected Ok, got {:?}", result);

        let instances = result.unwrap();
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].id, "101");
        assert_eq!(instances[0].ip, "10.0.0.1");
        assert_eq!(instances[0].provider, "hetzner");
        assert_eq!(instances[1].id, "102");
        assert_eq!(instances[1].ip, "10.0.0.2");
    }

    // -----------------------------------------------------------------------
    // 3.3.e — Error mapping table for HZ
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn error_mapping_401_auth() {
        let (mock_server, provider) = setup_hz().await;

        Mock::given(method("GET"))
            .and(path("/v1/datacenters"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let result = provider.validate_token("bad").await;
        assert!(matches!(result.unwrap_err(), CloudError::Auth(_)));
    }

    #[tokio::test]
    async fn error_mapping_403_auth() {
        let (mock_server, provider) = setup_hz().await;

        Mock::given(method("GET"))
            .and(path("/v1/datacenters"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&mock_server)
            .await;

        let result = provider.validate_token("bad").await;
        assert!(matches!(result.unwrap_err(), CloudError::Auth(_)));
    }

    #[tokio::test]
    async fn error_mapping_429_rate_limit() {
        let (mock_server, provider) = setup_hz().await;

        Mock::given(method("GET"))
            .and(path("/v1/datacenters"))
            .respond_with(
                ResponseTemplate::new(429).insert_header("retry-after", "5"),
            )
            .mount(&mock_server)
            .await;

        let result = provider.validate_token("token").await;
        match result.unwrap_err() {
            CloudError::RateLimit(secs) => assert_eq!(secs, 5),
            other => panic!("expected RateLimit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn error_mapping_503_provider() {
        let (mock_server, provider) = setup_hz().await;

        Mock::given(method("GET"))
            .and(path("/v1/datacenters"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&mock_server)
            .await;

        let result = provider.validate_token("token").await;
        assert!(matches!(result.unwrap_err(), CloudError::Provider(_)));
    }

    #[tokio::test]
    async fn error_mapping_404_provider() {
        let (mock_server, provider) = setup_hz().await;

        Mock::given(method("DELETE"))
            .and(path("/v1/servers/88888"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let result = provider.destroy_vps("88888", "token").await;
        assert!(matches!(result.unwrap_err(), CloudError::Provider(_)));
    }
}
