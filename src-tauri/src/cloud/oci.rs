use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use reqwest::Client;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::Pkcs1v15Sign;
use rsa::RsaPrivateKey;
use sha2::{Digest, Sha256};

use crate::cloud::{map_http_error, CloudError, CloudProvider, ProvisionParams, VpsInstance};

// ---------------------------------------------------------------------------
// Production code
// ---------------------------------------------------------------------------

/// SHA-256 DigestInfo prefix for PKCS#1 v1.5 (19 bytes, DER-encoded).
///
/// `30 31 30 0d 06 09 60 86 48 01 65 03 04 02 01 05 00 04 20`
const SHA256_DIGEST_INFO: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03,
    0x04, 0x02, 0x01, 0x05, 0x00, 0x04, 0x20,
];

/// OCI API base URL template — `{region}` is replaced at construction time.
const OCI_BASE_TEMPLATE: &str = "https://iaas.{region}.oraclecloud.com/20160918";

/// Parsed credentials from the OCI token JSON blob.
///
/// The stored "token" for OCI is a JSON object with these four fields.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OciCredentials {
    pub user_ocid: String,
    pub tenancy_ocid: String,
    pub key_fingerprint: String,
    pub private_key_pem: String,
}

/// Oracle Cloud Infrastructure provider.
///
/// # Auth
///
/// Unlike Bearer-based providers, OCI uses RSA-SHA256 request signing
/// (RFC 7235). Each trait method receives a `token` parameter that is a JSON
/// blob containing the user OCID, tenancy OCID, key fingerprint, and RSA
/// private key PEM.  The private key is used to sign every request.
pub struct OracleCloudProvider {
    client: Client,
    base_url: String,
    region: String,
    compartment_id: Option<String>,
}

impl OracleCloudProvider {
    /// Create a new provider targeting the production OCI API in `region`.
    pub fn new(region: &str, compartment_id: Option<String>) -> Result<Self, CloudError> {
        let base_url = OCI_BASE_TEMPLATE.replace("{region}", region);
        Self::with_base_url(&base_url, region, compartment_id)
    }

    /// Create a provider with a custom base URL (used for testing with wiremock).
    pub(crate) fn with_base_url(
        base_url: &str,
        region: &str,
        compartment_id: Option<String>,
    ) -> Result<Self, CloudError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(CloudError::Http)?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            region: region.to_string(),
            compartment_id,
        })
    }

    /// Parse the OCI token JSON blob into [`OciCredentials`].
    ///
    /// Validates that all four fields are present. Returns
    /// [`CloudError::Provider`] on parse failure.
    pub(crate) fn parse_oci_token(json_str: &str) -> Result<OciCredentials, CloudError> {
        serde_json::from_str::<OciCredentials>(json_str)
            .map_err(|e| CloudError::Provider(format!("invalid OCI token JSON: {e}")))
    }

    /// Build the full OCI `Authorization: Signature …` header value.
    ///
    /// 1. Constructs the signing string from method, URL, date, and body hash.
    /// 2. Signs with RSA-SHA256 (PKCS1v15) using the credential's private key.
    /// 3. Returns the complete `Authorization` header value.
    ///
    /// `date` and `body_hash` MUST match the values used in the `Date` and
    /// `x-content-sha256` request headers respectively; if they differ the
    /// server will reject the signature.
    pub(crate) fn build_signature(
        method: &str,
        url: &reqwest::Url,
        date: &str,
        body_hash: &str,
        creds: &OciCredentials,
    ) -> Result<String, CloudError> {
        let path = url.path();
        let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
        let request_target = format!("{} {}{}", method.to_lowercase(), path, query);
        let host = url.host_str().unwrap_or("iaas.oraclecloud.com");

        let signing_string = format!(
            "(request-target): {request_target}\n\
             host: {host}\n\
             date: {date}\n\
             x-content-sha256: {body_hash}"
        );

        let private_key = RsaPrivateKey::from_pkcs1_pem(&creds.private_key_pem)
            .map_err(|e| CloudError::Provider(format!("failed to parse RSA key: {e}")))?;

        // Build PKCS#1 v1.5 signature with SHA-256:
        //   DigestInfo (prefix) || SHA-256 hash → PKCS1v15 pad → RSA sign
        let hash = Sha256::digest(signing_string.as_bytes());
        let mut digest_info = Vec::with_capacity(SHA256_DIGEST_INFO.len() + 32);
        digest_info.extend_from_slice(&SHA256_DIGEST_INFO);
        digest_info.extend_from_slice(&hash);

        let scheme = Pkcs1v15Sign::new_unprefixed();
        let sig_bytes = private_key
            .sign(scheme, &digest_info)
            .map_err(|e| CloudError::Provider(format!("signing failed: {e}")))?;
        let sig_b64 =
            base64::engine::general_purpose::STANDARD.encode(&sig_bytes);

        let key_id = format!(
            "{}/{}/{}",
            creds.tenancy_ocid, creds.user_ocid, creds.key_fingerprint
        );

        Ok(format!(
            r#"Signature keyId="{key_id}",algorithm="rsa-sha256",headers="(request-target) host date x-content-sha256",signature="{sig_b64}""#
        ))
    }

    /// Produces an RFC 2822 date string for the current time
    /// (e.g. `"Mon, 06 Jul 2026 12:00:00 GMT"`).
    fn current_rfc2822() -> String {
        let dur = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self::epoch_to_rfc2822(dur.as_secs())
    }

    /// Convert Unix epoch seconds to an RFC 2822 date string.
    fn epoch_to_rfc2822(secs: u64) -> String {
        let days_since_epoch = secs / 86400;
        let day_secs = secs % 86400;
        let hours = day_secs / 3600;
        let minutes = (day_secs % 3600) / 60;
        let seconds = day_secs % 60;

        // Compute year/month/day from days since epoch (proleptic Gregorian).
        let mut y: i64 = 1970;
        let mut remaining = days_since_epoch as i64;

        loop {
            let days_in_year = if Self::is_leap(y) { 366 } else { 365 };
            if remaining < days_in_year {
                break;
            }
            remaining -= days_in_year;
            y += 1;
        }

        let month_days = if Self::is_leap(y) {
            [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        } else {
            [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        };

        let mut month_idx: usize = 0;
        for (i, &md) in month_days.iter().enumerate() {
            if remaining < md {
                month_idx = i;
                break;
            }
            remaining -= md;
        }

        let day = remaining + 1;
        let weekday_idx = ((days_since_epoch as i64 + 4) % 7) as usize;

        const WEEKDAYS: &[&str] = &[
            "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat",
        ];
        const MONTHS: &[&str] = &[
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep",
            "Oct", "Nov", "Dec",
        ];

        format!(
            "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
            WEEKDAYS[weekday_idx], day, MONTHS[month_idx], y, hours, minutes, seconds,
        )
    }

    fn is_leap(year: i64) -> bool {
        (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
    }

    // -- signed request helpers -----------------------------------------------

    /// Perform a signed GET request.
    async fn signed_get(
        &self,
        url: &reqwest::Url,
        creds: &OciCredentials,
    ) -> Result<reqwest::Response, CloudError> {
        let body: &[u8] = &[];
        let date = Self::current_rfc2822();
        let body_hash =
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body));
        let auth =
            Self::build_signature("get", url, &date, &body_hash, creds)?;

        self.client
            .get(url.as_str())
            .header("Date", &date)
            .header("x-content-sha256", &body_hash)
            .header("Authorization", &auth)
            .send()
            .await
            .map_err(CloudError::Http)
    }

    /// Perform a signed POST request with a JSON body.
    async fn signed_post(
        &self,
        url: &reqwest::Url,
        creds: &OciCredentials,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, CloudError> {
        let body_bytes = serde_json::to_vec(body)
            .map_err(|e| CloudError::Provider(format!("serialization error: {e}")))?;
        let date = Self::current_rfc2822();
        let body_hash =
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&body_bytes));
        let auth = Self::build_signature("post", url, &date, &body_hash, creds)?;

        self.client
            .post(url.as_str())
            .header("Date", &date)
            .header("x-content-sha256", &body_hash)
            .header("Content-Type", "application/json")
            .header("Authorization", &auth)
            .body(body_bytes)
            .send()
            .await
            .map_err(CloudError::Http)
    }

    /// Perform a signed DELETE request.
    async fn signed_delete(
        &self,
        url: &reqwest::Url,
        creds: &OciCredentials,
    ) -> Result<reqwest::Response, CloudError> {
        let body: &[u8] = &[];
        let date = Self::current_rfc2822();
        let body_hash =
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body));
        let auth = Self::build_signature("delete", url, &date, &body_hash, creds)?;

        self.client
            .delete(url.as_str())
            .header("Date", &date)
            .header("x-content-sha256", &body_hash)
            .header("Authorization", &auth)
            .send()
            .await
            .map_err(CloudError::Http)
    }
}

impl CloudProvider for OracleCloudProvider {
    async fn validate_token(&self, token: &str) -> Result<bool, CloudError> {
        let compartment_id = self
            .compartment_id
            .as_ref()
            .ok_or_else(|| CloudError::Auth("compartment_id not configured".to_string()))?;
        let creds = Self::parse_oci_token(token)?;

        let url = reqwest::Url::parse_with_params(
            &format!("{}/instances", self.base_url),
            &[("compartmentId", compartment_id.as_str()), ("limit", "1")],
        )
        .map_err(|e| CloudError::Provider(format!("invalid URL: {e}")))?;

        let response = self.signed_get(&url, &creds).await?;
        match response.status().as_u16() {
            200 => Ok(true),
            401 | 403 => Err(map_http_error(response).await),
            _ => Err(map_http_error(response).await),
        }
    }

    async fn create_vps(
        &self,
        params: &ProvisionParams,
        token: &str,
    ) -> Result<VpsInstance, CloudError> {
        let compartment_id = self
            .compartment_id
            .as_ref()
            .ok_or_else(|| CloudError::Auth("compartment_id not configured".to_string()))?;
        let creds = Self::parse_oci_token(token)?;

        let body = serde_json::json!({
            "compartmentId": compartment_id,
            "shape": params.size,
            "displayName": format!("vpn-{}", params.region),
            "imageId": params.image,
            "subnetId": "ocid1.subnet.oc1..default",
        });

        let url = reqwest::Url::parse(&format!("{}/instances", self.base_url))
            .map_err(|e| CloudError::Provider(format!("invalid URL: {e}")))?;

        let response = self.signed_post(&url, &creds, &body).await?;
        let status = response.status();
        if status.is_success() || status.as_u16() == 201 {
            let data: serde_json::Value = response.json().await.map_err(CloudError::Http)?;
            // OCI may wrap the response in `data` or return the instance directly.
            let instance = data
                .get("data")
                .or_else(|| data.get("instance"))
                .unwrap_or(&data);
            Ok(parse_instance(instance, &self.region))
        } else {
            Err(map_http_error(response).await)
        }
    }

    async fn destroy_vps(
        &self,
        instance_id: &str,
        token: &str,
    ) -> Result<(), CloudError> {
        let creds = Self::parse_oci_token(token)?;

        let url = reqwest::Url::parse(&format!("{}/instances/{}", self.base_url, instance_id))
            .map_err(|e| CloudError::Provider(format!("invalid URL: {e}")))?;

        let response = self.signed_delete(&url, &creds).await?;
        let status = response.status();
        if status.is_success() || status.as_u16() == 204 {
            Ok(())
        } else {
            Err(map_http_error(response).await)
        }
    }

    async fn list_vpss(&self, token: &str) -> Result<Vec<VpsInstance>, CloudError> {
        let compartment_id = self
            .compartment_id
            .as_ref()
            .ok_or_else(|| CloudError::Auth("compartment_id not configured".to_string()))?;
        let creds = Self::parse_oci_token(token)?;

        let url = reqwest::Url::parse_with_params(
            &format!("{}/instances", self.base_url),
            &[("compartmentId", compartment_id.as_str())],
        )
        .map_err(|e| CloudError::Provider(format!("invalid URL: {e}")))?;

        let response = self.signed_get(&url, &creds).await?;
        let status = response.status();
        if status.is_success() {
            let data: serde_json::Value = response.json().await.map_err(CloudError::Http)?;
            Ok(parse_instances_list(&data, &self.region))
        } else {
            Err(map_http_error(response).await)
        }
    }
}

// ---------------------------------------------------------------------------
// Response parsing helpers
// ---------------------------------------------------------------------------

/// Extract a [`VpsInstance`] from a single OCI instance JSON object.
fn parse_instance(value: &serde_json::Value, region: &str) -> VpsInstance {
    let ip = value["primaryVnic"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|vnic| vnic["privateIp"].as_str())
        .unwrap_or("0.0.0.0")
        .to_string();

    VpsInstance {
        id: value["id"].as_str().unwrap_or("").to_string(),
        provider: "oracle".to_string(),
        region: region.to_string(),
        ip,
        status: value["lifecycleState"].as_str().unwrap_or("").to_string(),
    }
}

/// Parse the OCI list instances response (`{"data": [...]}`).
fn parse_instances_list(data: &serde_json::Value, region: &str) -> Vec<VpsInstance> {
    data["data"]
        .as_array()
        .map(|arr| arr.iter().map(|v| parse_instance(v, region)).collect())
        .unwrap_or_default()
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // -----------------------------------------------------------------------
    // Test RSA key (2048-bit PKCS#1 PEM, generated for testing only)
    // -----------------------------------------------------------------------
    const TEST_RSA_PEM: &str = concat!(
        "-----BEGIN RSA PRIVATE KEY-----\n",
        "MIIEowIBAAKCAQEAsv3HrDGAAiAwz0aVGfZUoc+QlSV7/PfxMIQoiaiqBB3mnHbG\n",
        "jj83xHkTWbTvNxzzg0eAxON+XuxWfRLqKfGuW4uEISYVG3TwWGX0/Wat2cyKrR9l\n",
        "C+e98kbojnLUmqU/6JDQBQsr42vyuRpNMlthxz1ooxzw0e41LEOyVh4bs33RC8Ph\n",
        "Gc2vosOgaU9OOVyQm4kFpLugxE8mkrK8Eplt9rSgGkLgmkLWGh073byqm8WMzgQO\n",
        "MM/slkXBNDschL/AFkCwnY9Z3D2CSm8ciPi21keN8qRo1ElpHGLWPk8IWUxyjZLN\n",
        "5a0+YdWpVAmiIHVXXlRIyZ4+/AhUCwvgj70OZwIDAQABAoIBAEm2Az2hYPL/KLqj\n",
        "eZGohO/iF8ukFSx3Owdc1YjjQajSW38B1wELfb7WkaZ2wbCzpoDguGHcwdT7hR6a\n",
        "5H4DfmdKzE4ObdDR1ozA6CRW3a988XscG7PMasfUdb78ARvyg6AVyuTY1ekhMmMS\n",
        "NspPIbQ4UNgjefUqIRGqi021tniy3hhyTVg0w2B3RXbvM7ndCK9LRwvqt30I9rNR\n",
        "ua77JoNSOUVKRSRsqwoPMjDJu7VOUEptGKQeWJz4wWX+tUI42V8op4+iASzDRjxr\n",
        "86Z4z+RZIfJgoxFF3JedMFNYd93+CBOyS6E/0NACyLGhs/b/iTRHILJ6Fs2CYse7\n",
        "ZbaZTRkCgYEA4L+U+pK6p7UyJW8Lskua9mGb9k/NCKngLXZVi9M0lRqof4ppDNBR\n",
        "vmnz3gk3StmsSDy1jH6Aec1g96AM/SvE8+2pd20WRc9rS62iP5JVaayY05nUp8Kn\n",
        "lOF0LXCT1dVJon8uQw5j2D0BtXxhTQIY6BYqEmqzm2BKZ2NJeBAaI10CgYEAy+Ff\n",
        "2znL1BQ0l07xZLHn7+A2xto2fT48g3iJ37hG2Ym8bPsAcCOBE1/4Yb9tImDOq19L\n",
        "1zIFavTRHH4bVF8PCwcAu0c0N27Ayew8/R1fJVA4pF1SLtWLC+eYmXOHDeq1tKfX\n",
        "19Z8WB3emQ3hqm7BDaannk4yJ+xbJpmkvki5wJMCgYEAj+3hCH9DDffaP2LYCLym\n",
        "Zran3JvKYIv5xuOLcVo2yG4kDlmjYNNgJiNQS5d3U3YHANPwKCMzP82pFavn5ZJM\n",
        "NTK0Xoj7xIVK31I5H6ElFeG0lX5kU3MzQwMHFbqM0lofJ/Nuuv7SLj8Tgxg+b8Sy\n",
        "Ep9vHhA7KXwG6iMJf9xAAPkCgYAPF1CUFpQaz6AQ7xv5Gx4S6GLFl1NfM+MgzCRQ\n",
        "dgBwi7xxyKaApnAgcgMdoSC/4bCKiNRBSoeSIir0U/VL6nlflJYeRqf7zmvgxmbB\n",
        "SZJIXcbDi9DQfKf9KphmC2IcypnGlIHqjQrJLvTSGW/xwJ7zlrljg2A9Cka49bh3\n",
        "CUUOlwKBgBaqk9WsqqdUHmI+5zdA9lBybn6BfEHhKzj0TWBQRcNDSNNj2iYZ95Kf\n",
        "nr9cM8ni9hZNz53Izaf3jEJUb2WrpdV7FPhsjqjUyUX+92yCNWljfVMqNmy/8GZi\n",
        "7PMDuPmmfEiP07pOgxt8Xzhglb4EQGk3GPaU1j8V+rNUpWWy1hR6\n",
        "-----END RSA PRIVATE KEY-----\n",
    );

    /// Create a minimal `OciCredentials` using the test RSA key.
    fn test_creds() -> OciCredentials {
        OciCredentials {
            user_ocid: "ocid1.user.oc1..testuser".into(),
            tenancy_ocid: "ocid1.tenancy.oc1..testtenancy".into(),
            key_fingerprint: "20:3b:97:test".into(),
            private_key_pem: TEST_RSA_PEM.into(),
        }
    }

    /// Helper: start a wiremock server and attach an `OracleCloudProvider`.
    async fn setup_oci() -> (MockServer, OracleCloudProvider) {
        let mock_server = MockServer::start().await;
        let base_url = mock_server.uri();
        let provider = OracleCloudProvider::with_base_url(
            &base_url,
            "test-region",
            Some("ocid1.compartment.oc1..test".into()),
        )
        .unwrap();
        (mock_server, provider)
    }

    // -----------------------------------------------------------------------
    // 4.4.a — Known-answer signing format test
    // -----------------------------------------------------------------------
    #[test]
    fn signing_header_format() {
        let creds = test_creds();
        let url = reqwest::Url::parse("https://iaas.us-ashburn-1.oraclecloud.com/20160918/instances?compartmentId=ocid1.test&limit=1")
            .expect("valid test URL");
        let body: &[u8] = &[];
        let date = "Mon, 06 Jul 2026 12:00:00 GMT";
        let body_hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body));

        let auth = OracleCloudProvider::build_signature("get", &url, date, &body_hash, &creds)
            .expect("build_signature should succeed");

        // Verify the Authorization header format
        assert!(
            auth.starts_with("Signature keyId=\""),
            "should start with Signature keyId=\"...\", got: {auth}"
        );

        // keyId = "{tenancy}/{user}/{fingerprint}"
        let expected_key_id = format!(
            "{}/{}/{}",
            creds.tenancy_ocid, creds.user_ocid, creds.key_fingerprint
        );
        assert!(
            auth.contains(&format!("keyId=\"{expected_key_id}\"")),
            "should contain correct keyId"
        );

        // algorithm
        assert!(
            auth.contains(r#"algorithm="rsa-sha256""#),
            "should contain algorithm=\"rsa-sha256\""
        );

        // headers
        assert!(
            auth.contains(r#"headers="(request-target) host date x-content-sha256""#),
            "should contain the correct headers string"
        );

        // signature field — must be valid base64
        let sig_start = auth.find(r#"signature=""#).map(|i| i + 11);
        let sig_end = sig_start.and_then(|start| auth[start..].find('"').map(|end| start + end));
        let sig_b64 = sig_start
            .zip(sig_end)
            .map(|(s, e)| &auth[s..e])
            .unwrap_or("");
        assert!(!sig_b64.is_empty(), "signature value should not be empty");
        // Should be valid base64
        use base64::Engine;
        assert!(
            base64::engine::general_purpose::STANDARD.decode(sig_b64).is_ok(),
            "signature should be valid base64"
        );
    }

    // -----------------------------------------------------------------------
    // 4.4.b — Error mapping test
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn error_mapping_429_rate_limit() {
        let (mock_server, provider) = setup_oci().await;

        Mock::given(method("GET"))
            .and(path("/instances"))
            .respond_with(
                ResponseTemplate::new(429).insert_header("retry-after", "7"),
            )
            .mount(&mock_server)
            .await;

        let creds = test_creds();
        let token_json = serde_json::json!(&creds).to_string();

        let result = provider.validate_token(&token_json).await;

        match result.unwrap_err() {
            CloudError::RateLimit(secs) => assert_eq!(secs, 7),
            other => panic!("expected RateLimit(7), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn error_mapping_401_auth() {
        let (mock_server, provider) = setup_oci().await;

        Mock::given(method("GET"))
            .and(path("/instances"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let creds = test_creds();
        let token_json = serde_json::json!(&creds).to_string();

        let result = provider.validate_token(&token_json).await;

        assert!(
            matches!(result.unwrap_err(), CloudError::Auth(_)),
            "expected Auth error"
        );
    }

    #[tokio::test]
    async fn error_mapping_403_auth() {
        let (mock_server, provider) = setup_oci().await;

        Mock::given(method("GET"))
            .and(path("/instances"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&mock_server)
            .await;

        let creds = test_creds();
        let token_json = serde_json::json!(&creds).to_string();

        let result = provider.validate_token(&token_json).await;

        assert!(
            matches!(result.unwrap_err(), CloudError::Auth(_)),
            "expected Auth error for 403"
        );
    }

    // -----------------------------------------------------------------------
    // 4.4.c — Rate-limit via retry
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn rate_limit_retry_recovery() {
        let (mock_server, provider) = setup_oci().await;

        // First call: 429
        Mock::given(method("GET"))
            .and(path("/instances"))
            .respond_with(
                ResponseTemplate::new(429).insert_header("retry-after", "1"),
            )
            .with_priority(1)
            .up_to_n_times(1)
            .mount(&mock_server)
            .await;

        // Second call: 200
        Mock::given(method("GET"))
            .and(path("/instances"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let retry = crate::cloud::RetryCloudProvider::with_retries(provider, 2, 50);

        let creds = test_creds();
        let token_json = serde_json::json!(&creds).to_string();

        let result = retry.validate_token(&token_json).await;

        assert!(
            result.is_ok(),
            "expected Ok after rate-limit retry, got {:?}",
            result
        );
    }

    // -----------------------------------------------------------------------
    // 4.4.d — Core function: parse_oci_token validates all fields
    // -----------------------------------------------------------------------
    #[test]
    fn parse_oci_token_valid() {
        let json = r#"{
            "user_ocid": "ocid1.user.oc1..u",
            "tenancy_ocid": "ocid1.tenancy.oc1..t",
            "key_fingerprint": "20:3b:97:ab",
            "private_key_pem": "-----BEGIN RSA PRIVATE KEY-----\n...\n-----END RSA PRIVATE KEY-----"
        }"#;

        let creds = OracleCloudProvider::parse_oci_token(json).expect("should parse");
        assert_eq!(creds.user_ocid, "ocid1.user.oc1..u");
        assert_eq!(creds.tenancy_ocid, "ocid1.tenancy.oc1..t");
        assert_eq!(creds.key_fingerprint, "20:3b:97:ab");
        assert!(creds.private_key_pem.contains("RSA PRIVATE KEY"));
    }

    #[test]
    fn parse_oci_token_missing_fields() {
        let json = r#"{"user_ocid": "u"}"#;
        let result = OracleCloudProvider::parse_oci_token(json);
        assert!(result.is_err(), "expected error for missing fields");
    }

    // -----------------------------------------------------------------------
    // 4.4.e — Date formatter
    // -----------------------------------------------------------------------
    #[test]
    fn epoch_to_rfc2822_known() {
        // 1970-01-01 00:00:00 UTC was a Thursday
        let date = OracleCloudProvider::epoch_to_rfc2822(0);
        assert_eq!(date, "Thu, 01 Jan 1970 00:00:00 GMT");

        // 2024-01-01 00:00:00 UTC = 1704067200
        let date = OracleCloudProvider::epoch_to_rfc2822(1704067200);
        assert_eq!(date, "Mon, 01 Jan 2024 00:00:00 GMT");
    }

    #[test]
    fn epoch_to_rfc2822_leap_year() {
        // 2020-02-29 12:00:00 UTC = 1582977600
        let date = OracleCloudProvider::epoch_to_rfc2822(1582977600);
        assert_eq!(date, "Sat, 29 Feb 2020 12:00:00 GMT");
    }

    // -----------------------------------------------------------------------
    // 4.4.f — Validate token requires compartment_id
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn validate_token_no_compartment() {
        let provider =
            OracleCloudProvider::new("us-ashburn-1", None).expect("provider creation should work");

        let result = provider.validate_token("{}").await;
        match result.unwrap_err() {
            CloudError::Auth(msg) => {
                assert!(
                    msg.contains("compartment_id"),
                    "expected compartment_id error, got: {msg}"
                );
            }
            other => panic!("expected Auth error, got {other:?}"),
        }
    }
}
