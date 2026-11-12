//! HTTP and credentials, shared by every gateway.

use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use crate::use_cases::ports::SourceError;

/// Where a token comes from. The environment wins; the command is run
/// through `sh -c` and its first line of output is the token.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Credential {
    pub env: Option<String>,
    pub command: Option<String>,
    /// Run once when the source refuses the token, before asking `command`
    /// again: CLI logins keep short-lived tokens they renew on demand.
    pub renew: Option<String>,
}

impl Credential {
    fn resolve(&self) -> Result<String, SourceError> {
        if let Some(name) = &self.env
            && let Ok(value) = std::env::var(name)
            && !value.trim().is_empty()
        {
            return Ok(value.trim().to_string());
        }
        let Some(command) = &self.command else {
            return Err(SourceError::Auth(match &self.env {
                Some(name) => format!("${name} is not set"),
                None => "no token_env or token_command configured".to_string(),
            }));
        };
        let output = Command::new("sh")
            .args(["-c", command])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|error| SourceError::Auth(format!("token command failed: {error}")))?;
        let token = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if token.is_empty() {
            return Err(SourceError::Auth(
                "token command printed nothing".to_string(),
            ));
        }
        Ok(token)
    }
}

pub enum Auth<'a> {
    Bearer(&'a str),
    /// A ready `Authorization` header value.
    Raw(&'a str),
}

/// One source's connection: an HTTP agent and a token that is fetched
/// lazily, kept, and fetched again once when the source refuses it (CLI
/// tokens such as glab's expire and are refreshed by the command).
pub struct Session {
    agent: ureq::Agent,
    credential: Credential,
    token: Mutex<Option<String>>,
}

impl Session {
    pub fn new(credential: Credential) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("aki")
            .build()
            .into();
        Self {
            agent,
            credential,
            token: Mutex::new(None),
        }
    }

    fn token(&self) -> Result<String, SourceError> {
        let mut kept = self.token.lock().unwrap();
        if let Some(token) = kept.as_ref() {
            return Ok(token.clone());
        }
        let token = self.credential.resolve()?;
        *kept = Some(token.clone());
        Ok(token)
    }

    /// Runs `call` with the token; on a refusal, renews the token and tries
    /// once more.
    pub fn with_token<T>(
        &self,
        call: impl Fn(&str) -> Result<T, SourceError>,
    ) -> Result<T, SourceError> {
        match call(&self.token()?) {
            Err(SourceError::Auth(_)) => {
                *self.token.lock().unwrap() = None;
                if let Some(renew) = &self.credential.renew {
                    let _ = Command::new("sh")
                        .args(["-c", renew])
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
                call(&self.token()?)
            }
            other => other,
        }
    }

    pub fn get(&self, url: &str, auth: Auth) -> Result<Value, SourceError> {
        let request = self
            .agent
            .get(url)
            .header("Accept", "application/json")
            .header("Authorization", header(auth));
        read(request.call())
    }

    pub fn post(&self, url: &str, auth: Auth, body: &Value) -> Result<Value, SourceError> {
        let request = self
            .agent
            .post(url)
            .header("Accept", "application/json")
            .header("Authorization", header(auth));
        read(request.send_json(body))
    }

    /// A GraphQL call: unwraps `data`, turns `errors` into failures.
    pub fn graphql(
        &self,
        url: &str,
        token: &str,
        query: &str,
        variables: Value,
    ) -> Result<Value, SourceError> {
        let body = serde_json::json!({ "query": query, "variables": variables });
        let mut answer = self.post(url, Auth::Bearer(token), &body)?;
        if let Some(message) = answer["errors"][0]["message"].as_str() {
            // Partial data with errors is still an error: half a board lies.
            return Err(SourceError::Unexpected(message.to_string()));
        }
        Ok(answer["data"].take())
    }
}

fn header(auth: Auth) -> String {
    match auth {
        Auth::Bearer(token) => format!("Bearer {token}"),
        Auth::Raw(value) => value.to_string(),
    }
}

fn read(
    response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Value, SourceError> {
    match response {
        Ok(mut response) => response
            .body_mut()
            .read_json::<Value>()
            .map_err(|error| SourceError::Unexpected(error.to_string())),
        Err(ureq::Error::StatusCode(401)) => Err(SourceError::Auth("token refused (401)".into())),
        Err(ureq::Error::StatusCode(403)) => {
            Err(SourceError::Auth("access forbidden (403)".into()))
        }
        Err(ureq::Error::StatusCode(code)) => Err(SourceError::Unreachable(format!("HTTP {code}"))),
        Err(error) => Err(SourceError::Unreachable(error.to_string())),
    }
}

/// Whether `text` calls `@name`, and not a longer handle that starts the same.
pub fn mentions(text: &str, name: &str) -> bool {
    let handle = format!("@{name}");
    text.match_indices(&handle).any(|(start, _)| {
        text[start + handle.len()..]
            .chars()
            .next()
            .is_none_or(|next| !(next.is_alphanumeric() || next == '_' || next == '-'))
    })
}

pub fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let block = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let bits = u32::from(block[0]) << 16 | u32::from(block[1]) << 8 | u32::from(block[2]);
        for position in 0..4 {
            if position <= chunk.len() {
                let index = (bits >> (18 - 6 * position)) & 0x3f;
                output.push(ALPHABET[index as usize] as char);
            } else {
                output.push('=');
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"me@x.io:tok"), "bWVAeC5pbzp0b2s=");
    }

    #[test]
    fn mentions_whole_handles_only() {
        assert!(mentions("ping @sam.doe please", "sam.doe"));
        assert!(mentions("@sam", "sam"));
        assert!(!mentions("ping @samantha", "sam"));
        assert!(!mentions("sam", "sam"));
    }
}
