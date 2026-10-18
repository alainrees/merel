//! OAuth 2.0 authorization code flow with PKCE for a desktop app (RFC 8252).
//!
//! The person signs in in their own browser; the provider sends the code back to a listener
//! on `127.0.0.1` that exists only for this one sign-in. No embedded web view, so the app never
//! sees the password. The refresh token is the only thing kept, in the system keychain.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use base64::Engine as _;
use mailcraft_secrets::Secret;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("could not open a port on this computer for the sign-in: {0}")]
    Listen(String),
    #[error("could not open the browser: {0}")]
    Browser(String),
    #[error("the sign-in was cancelled or timed out")]
    Cancelled,
    #[error("the sign-in came back with the wrong state; try again")]
    State,
    #[error("the provider refused: {0}")]
    Provider(String),
    #[error("the token request failed: {0}")]
    Http(String),
}

/// One provider's endpoints and this app's registration with it.
#[derive(Clone, Debug)]
pub struct Client {
    pub client_id: String,
    /// Google issues one for desktop apps; Microsoft public clients have none.
    pub client_secret: Option<Secret>,
    pub auth_url: String,
    pub token_url: String,
    pub scope: String,
}

/// What a finished sign-in yields.
#[derive(Clone, Debug)]
pub struct Tokens {
    pub access_token: Secret,
    pub refresh_token: Option<Secret>,
    pub expires_in: Duration,
}

/// How long the browser gets before the sign-in counts as cancelled.
pub const TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Installs the process-wide rustls crypto provider once; ureq, lettre and imap all share it.
pub fn install_crypto() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

fn random_urlsafe(bytes: usize) -> Result<String, OAuthError> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| OAuthError::Listen(e.to_string()))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf))
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The query string of a redirect, split into pairs and percent-decoded.
fn query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            Some((decode(k)?, decode(v)?))
        })
        .collect()
}

fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// The authorization URL for `client`, with PKCE and a state value.
pub struct Request {
    pub url: String,
    verifier: String,
    state: String,
    redirect_uri: String,
}

impl Client {
    fn request(&self, redirect_uri: &str) -> Result<Request, OAuthError> {
        let verifier = random_urlsafe(48)?;
        let state = random_urlsafe(24)?;
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
            self.auth_url,
            encode(&self.client_id),
            encode(redirect_uri),
            encode(&self.scope),
            encode(&state),
            encode(&challenge)
        );
        Ok(Request { url, verifier, state, redirect_uri: redirect_uri.to_string() })
    }

    /// Runs the whole sign-in: opens the browser, waits for the redirect, exchanges the code.
    /// `open_browser` lets tests skip the real browser.
    pub fn sign_in(&self, open_browser: bool) -> Result<Tokens, OAuthError> {
        install_crypto();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| OAuthError::Listen(e.to_string()))?;
        let port = listener.local_addr().map_err(|e| OAuthError::Listen(e.to_string()))?.port();
        let request = self.request(&format!("http://127.0.0.1:{port}/"))?;
        if open_browser {
            open::that_detached(&request.url).map_err(|e| OAuthError::Browser(e.to_string()))?;
        }
        let code = wait_for_code(&listener, &request.state, TIMEOUT)?;
        self.exchange(&request, &code)
    }

    fn exchange(&self, request: &Request, code: &str) -> Result<Tokens, OAuthError> {
        let mut form = format!(
            "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
            encode(code),
            encode(&request.redirect_uri),
            encode(&self.client_id),
            encode(&request.verifier)
        );
        if let Some(secret) = &self.client_secret {
            form.push_str(&format!("&client_secret={}", encode(secret.expose())));
        }
        self.token_request(form)
    }

    /// A fresh access token from a refresh token.
    pub fn refresh(&self, refresh_token: &Secret) -> Result<Tokens, OAuthError> {
        install_crypto();
        let mut form = format!("grant_type=refresh_token&refresh_token={}&client_id={}", encode(refresh_token.expose()), encode(&self.client_id));
        if let Some(secret) = &self.client_secret {
            form.push_str(&format!("&client_secret={}", encode(secret.expose())));
        }
        if self.scope.contains("outlook.office.com") {
            form.push_str(&format!("&scope={}", encode(&self.scope)));
        }
        self.token_request(form)
    }

    fn token_request(&self, form: String) -> Result<Tokens, OAuthError> {
        #[derive(Deserialize)]
        struct Reply {
            access_token: Option<String>,
            refresh_token: Option<String>,
            expires_in: Option<u64>,
            error: Option<String>,
            error_description: Option<String>,
        }
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder().timeout_global(Some(Duration::from_secs(30))).http_status_as_error(false).build(),
        );
        let mut response = agent
            .post(&self.token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .send(form.as_bytes())
            .map_err(|e| OAuthError::Http(e.to_string()))?;
        let reply: Reply = response.body_mut().with_config().limit(64 * 1024).read_json().map_err(|e| OAuthError::Http(e.to_string()))?;
        if let Some(err) = reply.error {
            let desc = reply.error_description.unwrap_or_default();
            return Err(OAuthError::Provider(format!("{err} {desc}").trim().chars().take(200).collect()));
        }
        let access = reply.access_token.ok_or_else(|| OAuthError::Provider("no access token in the reply".into()))?;
        Ok(Tokens {
            access_token: Secret::new(access),
            refresh_token: reply.refresh_token.map(Secret::new),
            expires_in: Duration::from_secs(reply.expires_in.unwrap_or(3600)),
        })
    }
}

/// Waits for the browser to hit the loopback listener and returns the authorization code.
fn wait_for_code(listener: &TcpListener, state: &str, timeout: Duration) -> Result<String, OAuthError> {
    listener.set_nonblocking(true).map_err(|e| OAuthError::Listen(e.to_string()))?;
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(result) = handle(stream, state) {
                    return result;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    return Err(OAuthError::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(OAuthError::Listen(e.to_string())),
        }
    }
}

const PAGE_OK: &str = "<!doctype html><html lang=\"nl\"><meta charset=\"utf-8\"><title>Mailcraft</title><body style=\"font-family:sans-serif;padding:3em\"><h1>Gelukt</h1><p>Je bent aangemeld. Je kunt dit venster sluiten en teruggaan naar Mailcraft.</p></body></html>";
const PAGE_ERR: &str = "<!doctype html><html lang=\"nl\"><meta charset=\"utf-8\"><title>Mailcraft</title><body style=\"font-family:sans-serif;padding:3em\"><h1>Niet gelukt</h1><p>Het aanmelden is niet gelukt. Ga terug naar Mailcraft en probeer het opnieuw.</p></body></html>";

/// One HTTP request on the loopback listener. `None` means it was not the redirect (a favicon
/// request, a stray connection) and the wait goes on.
fn handle(mut stream: TcpStream, state: &str) -> Option<Result<String, OAuthError>> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).ok()?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let line = head.lines().next()?;
    let path = line.strip_prefix("GET ")?.split(' ').next()?;
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    if query.is_empty() {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }
    let pairs = query_pairs(query);
    let get = |k: &str| pairs.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    let result = if get("state").as_deref() != Some(state) {
        Err(OAuthError::State)
    } else if let Some(err) = get("error") {
        Err(OAuthError::Provider(err.chars().take(100).collect()))
    } else if let Some(code) = get("code") {
        Ok(code)
    } else {
        Err(OAuthError::Provider("no code in the redirect".into()))
    };
    let page = if result.is_ok() { PAGE_OK } else { PAGE_ERR };
    let _ = stream.write_all(
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len())
            .as_bytes(),
    );
    Some(result)
}

/// The SASL XOAUTH2 initial response for IMAP and SMTP.
pub fn xoauth2(user: &str, access_token: &Secret) -> String {
    format!("user={user}\x01auth=Bearer {}\x01\x01", access_token.expose())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> Client {
        Client {
            client_id: "id".into(),
            client_secret: None,
            auth_url: "https://auth.example/a".into(),
            token_url: "https://auth.example/t".into(),
            scope: "mail x".into(),
        }
    }

    #[test]
    fn request_carries_pkce_and_state() {
        let r = client().request("http://127.0.0.1:1234/").unwrap();
        assert!(r.url.starts_with(
            "https://auth.example/a?response_type=code&client_id=id&redirect_uri=http%3A%2F%2F127.0.0.1%3A1234%2F&scope=mail%20x&state="
        ));
        assert!(r.url.contains("code_challenge_method=S256"));
        assert!(r.verifier.len() >= 43);
        assert_ne!(r.state, client().request("x").unwrap().state);
    }

    #[test]
    fn loopback_redirect_yields_the_code_and_checks_state() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let hit = std::thread::spawn(move || {
            for path in ["/favicon.ico", "/?state=wrong&code=1", "/?state=s%201&code=abc%2Fdef"] {
                let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
                s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).unwrap();
                let mut reply = String::new();
                let _ = s.read_to_string(&mut reply);
                if path.contains("wrong") {
                    assert!(reply.contains("Niet gelukt"));
                }
            }
        });
        let listener2 = listener.try_clone().unwrap();
        let first = wait_for_code(&listener2, "s 1", Duration::from_secs(5));
        assert!(matches!(first, Err(OAuthError::State)));
        let second = wait_for_code(&listener2, "s 1", Duration::from_secs(5)).unwrap();
        assert_eq!(second, "abc/def");
        hit.join().unwrap();
    }

    #[test]
    fn xoauth2_shape() {
        assert_eq!(xoauth2("a@b.nl", &Secret::new("tok")), "user=a@b.nl\x01auth=Bearer tok\x01\x01");
    }
}
