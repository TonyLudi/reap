use std::collections::BTreeMap;
use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct OkxCredentials {
    api_key: String,
    secret_key: String,
    passphrase: String,
}

impl OkxCredentials {
    pub fn new(
        api_key: impl Into<String>,
        secret_key: impl Into<String>,
        passphrase: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            secret_key: secret_key.into(),
            passphrase: passphrase.into(),
        }
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    pub fn passphrase(&self) -> &str {
        &self.passphrase
    }
}

impl fmt::Debug for OkxCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OkxCredentials")
            .field("api_key", &"[REDACTED]")
            .field("secret_key", &"[REDACTED]")
            .field("passphrase", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid HMAC key")]
    InvalidKey,
    #[error("failed to serialize login payload: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

impl HttpMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SignedRequest {
    pub method: HttpMethod,
    pub path: String,
    pub body: String,
    pub headers: BTreeMap<String, String>,
}

impl fmt::Debug for SignedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers = self
            .headers
            .keys()
            .map(|name| (name.as_str(), "[REDACTED]"))
            .collect::<BTreeMap<_, _>>();
        formatter
            .debug_struct("SignedRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("body", &self.body)
            .field("headers", &headers)
            .finish()
    }
}

#[derive(Clone)]
pub struct OkxSigner {
    credentials: OkxCredentials,
    demo_trading: bool,
}

impl OkxSigner {
    pub fn new(credentials: OkxCredentials, demo_trading: bool) -> Self {
        Self {
            credentials,
            demo_trading,
        }
    }

    pub fn credentials(&self) -> &OkxCredentials {
        &self.credentials
    }

    pub fn signature(
        &self,
        timestamp: &str,
        method: HttpMethod,
        path: &str,
        body: &str,
    ) -> Result<String, AuthError> {
        let mut mac = HmacSha256::new_from_slice(self.credentials.secret_key.as_bytes())
            .map_err(|_| AuthError::InvalidKey)?;
        mac.update(timestamp.as_bytes());
        mac.update(method.as_str().as_bytes());
        mac.update(path.as_bytes());
        mac.update(body.as_bytes());
        Ok(STANDARD.encode(mac.finalize().into_bytes()))
    }

    pub fn sign_request(
        &self,
        timestamp: &str,
        method: HttpMethod,
        path: impl Into<String>,
        body: impl Into<String>,
    ) -> Result<SignedRequest, AuthError> {
        let path = path.into();
        let body = body.into();
        let mut headers = BTreeMap::from([
            (
                "OK-ACCESS-KEY".to_string(),
                self.credentials.api_key.clone(),
            ),
            (
                "OK-ACCESS-PASSPHRASE".to_string(),
                self.credentials.passphrase.clone(),
            ),
            ("OK-ACCESS-TIMESTAMP".to_string(), timestamp.to_string()),
            (
                "OK-ACCESS-SIGN".to_string(),
                self.signature(timestamp, method, &path, &body)?,
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ]);
        if self.demo_trading {
            headers.insert("x-simulated-trading".to_string(), "1".to_string());
        }
        Ok(SignedRequest {
            method,
            path,
            body,
            headers,
        })
    }

    pub fn websocket_login(&self, timestamp_seconds: &str) -> Result<String, AuthError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct LoginArg<'a> {
            api_key: &'a str,
            passphrase: &'a str,
            timestamp: &'a str,
            sign: String,
        }

        #[derive(Serialize)]
        struct Login<'a> {
            op: &'static str,
            args: [LoginArg<'a>; 1],
        }

        let sign = self.signature(timestamp_seconds, HttpMethod::Get, "/users/self/verify", "")?;
        Ok(serde_json::to_string(&Login {
            op: "login",
            args: [LoginArg {
                api_key: self.credentials.api_key(),
                passphrase: self.credentials.passphrase(),
                timestamp: timestamp_seconds,
                sign,
            }],
        })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_exact_prehash_and_redacts_credentials() {
        let signer = OkxSigner::new(
            OkxCredentials::new("key", "actual-secret-material", "pass"),
            false,
        );
        let signature = signer
            .signature(
                "2020-12-08T09:08:57.715Z",
                HttpMethod::Get,
                "/api/v5/account/balance?ccy=BTC",
                "",
            )
            .unwrap();

        assert_eq!(signature, "GpEEDu/mXv8KcL4vAOIcuA7n2MGtmaxOnshqzPQ3tQI=");
        assert!(!format!("{:?}", signer.credentials()).contains("actual-secret-material"));
    }

    #[test]
    fn demo_header_and_websocket_login_are_generated() {
        let signer = OkxSigner::new(OkxCredentials::new("key", "secret", "pass"), true);
        let request = signer
            .sign_request("time", HttpMethod::Post, "/path", "{}")
            .unwrap();
        assert_eq!(request.headers["x-simulated-trading"], "1");

        let login: serde_json::Value =
            serde_json::from_str(&signer.websocket_login("1538054050").unwrap()).unwrap();
        assert_eq!(login["op"], "login");
        assert_eq!(login["args"][0]["apiKey"], "key");
        assert!(!format!("{request:?}").contains("secret"));
        assert!(!format!("{request:?}").contains("pass"));
    }
}
