use std::time::Duration;

use reqwest::Client;

use crate::cloud::{map_http_error, CloudError, CloudProvider, ProvisionParams, VpsInstance};

// ---------------------------------------------------------------------------
// Production code
// ---------------------------------------------------------------------------

const DO_API_BASE: &str = "https://api.digitalocean.com/v2";

/// DigitalOcean Droplets API provider.
///
/// # Auth
/// Uses `Authorization: Bearer {token}` on every request. The token is never
/// stored on the struct — it is passed as a `&str` parameter to each method.
pub struct DigitalOceanProvider {
    client: Client,
    base_url: String,
}

impl DigitalOceanProvider {
    /// Create a new provider targeting the production DO API.
    pub fn new() -> Result<Self, CloudError> {
        Self::with_base_url(DO_API_BASE)
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

impl CloudProvider for DigitalOceanProvider {
    async fn validate_token(&self, token: &str) -> Result<bool, CloudError> {
        let response = self.get("/v2/account", token).await?;
        if response.status().is_success() {
            Ok(true)
        } else {
            Err(map_http_error(response).await)        }
    }

    async fn create_vps(&self, params: &ProvisionParams, token: &str) -> Result<VpsInstance, CloudError> {
        let body = serde_json::json!({
            "name": format!("vpn-{}", params.region),
            "region": params.region,
            "size": params.size,
            "image": params.image,
        });
        let response = self.post("/v2/droplets", token, body).await?;
        let status = response.status();
        if status.is_success() || status.as_u16() == 202 {
            let data: serde_json::Value = response.json().await.map_err(CloudError::Http)?;
            Ok(parse_droplet(&data["droplet"], "digitalocean"))
        } else {
            Err(map_http_error(response).await)        }
    }

    async fn destroy_vps(&self, instance_id: &str, token: &str) -> Result<(), CloudError> {
        let response = self.delete(&format!("/v2/droplets/{}", instance_id), token).await?;
        if response.status().is_success() || response.status().as_u16() == 204 {
            Ok(())
        } else {
            Err(map_http_error(response).await)        }
    }

    async fn list_vpss(&self, token: &str) -> Result<Vec<VpsInstance>, CloudError> {
        let response = self.get("/v2/droplets", token).await?;
        let status = response.status();
        if status.is_success() {
            let data: serde_json::Value = response.json().await.map_err(CloudError::Http)?;
            let droplets = data["droplets"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|d| parse_droplet(d, "digitalocean"))
                        .collect()
                })
                .unwrap_or_default();
            Ok(droplets)
        } else {
            Err(map_http_error(response).await)        }
    }
}

// ---------------------------------------------------------------------------
// Response parsing helper
// ---------------------------------------------------------------------------

/// Extract a `VpsInstance` from a DO Droplet JSON object.
fn parse_droplet(value: &serde_json::Value, provider: &str) -> VpsInstance {
    let ip = value["networks"]["v4"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|n| n["type"] == "public")
                .or_else(|| arr.first())
        })
        .and_then(|n| n["ip_address"].as_str())
        .unwrap_or("")
        .to_string();

    VpsInstance {
        id: value["id"].to_string(),
        provider: provider.to_string(),
        region: value["region"]["slug"].as_str().unwrap_or("fra1").to_string(),
        ip,
        status: value["status"].as_str().unwrap_or("new").to_string(),
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

    /// Helper: start a wiremock server and attach a `DigitalOceanProvider`.
    async fn setup_do() -> (MockServer, DigitalOceanProvider) {
        let mock_server = MockServer::start().await;
        let base_url = mock_server.uri();
        let provider = DigitalOceanProvider::with_base_url(&base_url).unwrap();
        (mock_server, provider)
    }

    // -----------------------------------------------------------------------
    // 3.3.a — Token validation (valid / invalid)
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn validate_token_valid() {
        let (mock_server, provider) = setup_do().await;

        Mock::given(method("GET"))
            .and(path("/v2/account"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let result = provider.validate_token("do-test-token").await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn validate_token_invalid() {
        let (mock_server, provider) = setup_do().await;

        Mock::given(method("GET"))
            .and(path("/v2/account"))
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
    // 3.3.b — Create droplet
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn create_droplet_success() {
        let (mock_server, provider) = setup_do().await;

        let body = serde_json::json!({
            "droplet": {
                "id": 123456,
                "name": "test-droplet",
                "region": { "slug": "fra1" },
                "networks": { "v4": [{ "ip_address": "10.0.0.1", "type": "public" }] },
                "status": "active"
            }
        });

        Mock::given(method("POST"))
            .and(path("/v2/droplets"))
            .respond_with(ResponseTemplate::new(202).set_body_json(body))
            .mount(&mock_server)
            .await;

        let params = ProvisionParams {
            provider: "digitalocean".into(),
            region: "fra1".into(),
            size: "s-1vcpu-1gb".into(),
            image: "ubuntu-24-04-x64".into(),
        };

        let result = provider.create_vps(&params, "do-token").await;
        assert!(result.is_ok(), "expected Ok, got {:?}", result);

        let instance = result.unwrap();
        assert_eq!(instance.provider, "digitalocean");
        assert_eq!(instance.ip, "10.0.0.1");
        assert_eq!(instance.status, "active");
        assert_eq!(instance.region, "fra1");
    }

    // -----------------------------------------------------------------------
    // 3.3.c — Destroy droplet
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn destroy_droplet_success() {
        let (mock_server, provider) = setup_do().await;

        Mock::given(method("DELETE"))
            .and(path("/v2/droplets/98765"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&mock_server)
            .await;

        let result = provider.destroy_vps("98765", "do-token").await;
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // 3.3.d — List droplets
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn list_droplets_success() {
        let (mock_server, provider) = setup_do().await;

        let body = serde_json::json!({
            "droplets": [
                {
                    "id": 111,
                    "name": "droplet-1",
                    "region": { "slug": "fra1" },
                    "networks": { "v4": [{ "ip_address": "10.0.0.1", "type": "public" }] },
                    "status": "active"
                },
                {
                    "id": 222,
                    "name": "droplet-2",
                    "region": { "slug": "nyc1" },
                    "networks": { "v4": [{ "ip_address": "10.0.0.2", "type": "public" }] },
                    "status": "off"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/v2/droplets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&mock_server)
            .await;

        let result = provider.list_vpss("do-token").await;
        assert!(result.is_ok(), "expected Ok, got {:?}", result);

        let instances = result.unwrap();
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].id, "111");
        assert_eq!(instances[0].ip, "10.0.0.1");
        assert_eq!(instances[0].provider, "digitalocean");
        assert_eq!(instances[1].id, "222");
        assert_eq!(instances[1].ip, "10.0.0.2");
    }

    // -----------------------------------------------------------------------
    // 3.3.e — Error mapping table for DO
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn error_mapping_401_auth() {
        let (mock_server, provider) = setup_do().await;

        Mock::given(method("GET"))
            .and(path("/v2/account"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let result = provider.validate_token("bad-token").await;
        assert!(matches!(result.unwrap_err(), CloudError::Auth(_)));
    }

    #[tokio::test]
    async fn error_mapping_403_auth() {
        let (mock_server, provider) = setup_do().await;

        Mock::given(method("GET"))
            .and(path("/v2/account"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&mock_server)
            .await;

        let result = provider.validate_token("forbidden-token").await;
        assert!(matches!(result.unwrap_err(), CloudError::Auth(_)));
    }

    #[tokio::test]
    async fn error_mapping_429_rate_limit() {
        let (mock_server, provider) = setup_do().await;

        Mock::given(method("GET"))
            .and(path("/v2/account"))
            .respond_with(
                ResponseTemplate::new(429).insert_header("retry-after", "3"),
            )
            .mount(&mock_server)
            .await;

        let result = provider.validate_token("token").await;
        match result.unwrap_err() {
            CloudError::RateLimit(secs) => assert_eq!(secs, 3),
            other => panic!("expected RateLimit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn error_mapping_503_provider() {
        let (mock_server, provider) = setup_do().await;

        Mock::given(method("GET"))
            .and(path("/v2/account"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&mock_server)
            .await;

        let result = provider.validate_token("token").await;
        assert!(matches!(result.unwrap_err(), CloudError::Provider(_)));
    }

    #[tokio::test]
    async fn error_mapping_404_provider() {
        let (mock_server, provider) = setup_do().await;

        Mock::given(method("DELETE"))
            .and(path("/v2/droplets/99999"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let result = provider.destroy_vps("99999", "token").await;
        assert!(matches!(result.unwrap_err(), CloudError::Provider(_)));
    }
}
