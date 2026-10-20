//! Pictures on the internet, fetched only when the person asks for them, for one message at
//! a time. Nothing loads on its own: the reading pane shows a placeholder for every remote
//! picture until "Afbeeldingen laden" is clicked, which tells the sender the message was
//! opened and is therefore the person's call.
//!
//! Each fetch runs on its own thread with `ureq`: `https` only (a redirect to `http` fails),
//! a short timeout, a size cap, and the reply must say it is an image. Bytes are kept in
//! memory for the message that is open and dropped when another one opens.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mailcraft_html::{Block, Document, ImageSource, Inline};

/// The largest picture fetched, in bytes.
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
/// How many remote pictures of one message are fetched at most.
pub const MAX_IMAGES: usize = 40;
const TIMEOUT: Duration = Duration::from_secs(20);

/// Where one picture stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Load {
    Pending,
    Ready(Arc<[u8]>),
    Failed,
}

/// What is known about the pictures of one message, by URL.
pub type Loads = HashMap<String, Load>;

#[derive(Default)]
pub struct RemoteImages {
    /// The message whose pictures may load.
    allowed: Option<String>,
    loads: Arc<Mutex<Loads>>,
}

impl RemoteImages {
    /// Whether the pictures of `message_id` were allowed to load.
    pub fn allowed(&self, message_id: &str) -> bool {
        self.allowed.as_deref() == Some(message_id)
    }

    /// Drops everything fetched; for when another message opens.
    pub fn reset(&mut self) {
        self.allowed = None;
        self.loads = Arc::new(Mutex::new(HashMap::new()));
    }

    /// A copy of the state for drawing this frame, or `None` while nothing is allowed.
    pub fn snapshot(&self, message_id: &str) -> Option<Loads> {
        if !self.allowed(message_id) {
            return None;
        }
        Some(self.loads.lock().map(|l| l.clone()).unwrap_or_default())
    }

    /// Allows the pictures of `message_id` in `doc` and starts fetching them; the window is
    /// repainted as each one arrives.
    pub fn allow(&mut self, ctx: &egui::Context, message_id: &str, doc: &Document) {
        self.reset();
        self.allowed = Some(message_id.to_string());
        let urls = remote_urls(doc);
        let Ok(mut loads) = self.loads.lock() else { return };
        for url in urls.into_iter().take(MAX_IMAGES) {
            if loads.contains_key(&url) {
                continue;
            }
            if !fetchable(&url) {
                loads.insert(url, Load::Failed);
                continue;
            }
            loads.insert(url.clone(), Load::Pending);
            let shared = Arc::clone(&self.loads);
            let ctx = ctx.clone();
            let target = url.clone();
            let spawned = std::thread::Builder::new().name("remote-image".into()).spawn(move || {
                let result = fetch(&target);
                if let Err(e) = &result {
                    log::info!("remote picture not loaded: {e}");
                }
                if let Ok(mut l) = shared.lock()
                    && l.get(&target) == Some(&Load::Pending)
                {
                    l.insert(target, result.map(Load::Ready).unwrap_or(Load::Failed));
                }
                ctx.request_repaint();
            });
            if spawned.is_err() {
                loads.insert(url, Load::Failed);
            }
        }
    }
}

/// Whether `url` is one Mailcraft is willing to fetch: `https`, no control characters.
pub fn fetchable(url: &str) -> bool {
    url.starts_with("https://") && url.len() <= 2048 && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// Every remote picture URL in `doc`, in reading order, each once.
pub fn remote_urls(doc: &Document) -> Vec<String> {
    fn walk(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                    for i in inlines {
                        if let Inline::Image(img) = i
                            && let ImageSource::Remote(url) = &img.source
                            && !out.contains(url)
                        {
                            out.push(url.clone());
                        }
                    }
                }
                Block::Quote(inner) => walk(inner, out),
                Block::List { items, .. } => items.iter().for_each(|item| walk(item, out)),
                Block::Table(rows) => rows.iter().flatten().for_each(|c| walk(&c.blocks, out)),
                Block::Code(_) | Block::Rule => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.blocks, &mut out);
    out
}

/// Fetches one picture. `Err` carries the reason for the log only; the person sees a
/// placeholder.
fn fetch(url: &str) -> Result<Arc<[u8]>, String> {
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .tls_config(tls())
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .https_only(true)
            .max_redirects(3)
            .user_agent("Mailcraft")
            .build(),
    );
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let is_image = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.trim_start().to_ascii_lowercase().starts_with("image/"));
    if !is_image {
        return Err("not an image".into());
    }
    let bytes = response.body_mut().with_config().limit(MAX_IMAGE_BYTES).read_to_vec().map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Err("empty".into());
    }
    Ok(Arc::from(bytes))
}

/// HTTPS checked by the computer's own certificate store (Keychain on macOS), with aws-lc-rs
/// like the rest of Mailcraft. ureq has no root certificates of its own in this build: an
/// agent without this panics on its first request.
pub(crate) fn tls() -> ureq::tls::TlsConfig {
    ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_are_collected_once_and_only_https_is_fetchable() {
        let doc = mailcraft_html::parse(
            r#"<p><img src="https://a.example/1.png"> <img src="https://a.example/1.png"></p>
            <blockquote><img src="http://a.example/plain.png"></blockquote>
            <ul><li><img src="https://b.example/2.gif" alt="x"></li></ul>
            <table><tr><td>a</td><td><img src="https://c.example/3.jpg"></td></tr></table>
            <img src="cid:part1"> <img src="data:image/png;base64,iVBORw0KGgo=">"#,
        );
        let urls = remote_urls(&doc);
        assert_eq!(urls, ["https://a.example/1.png", "http://a.example/plain.png", "https://b.example/2.gif", "https://c.example/3.jpg"]);
        assert!(fetchable("https://a.example/1.png"));
        assert!(!fetchable("http://a.example/plain.png"), "plain http never, whatever the message says");
        assert!(!fetchable("https://a.example/1 .png"));
    }

    #[test]
    fn allowing_is_per_message_and_a_reset_forgets() {
        let doc = mailcraft_html::parse(r#"<p><img src="http://a.example/plain.png"></p>"#);
        let ctx = egui::Context::default();
        let mut r = RemoteImages::default();
        assert!(r.snapshot("m1").is_none());
        r.allow(&ctx, "m1", &doc);
        assert!(r.allowed("m1") && !r.allowed("m2"));
        assert_eq!(r.snapshot("m1").unwrap().get("http://a.example/plain.png"), Some(&Load::Failed), "refused without a request");
        r.reset();
        assert!(r.snapshot("m1").is_none());
    }
}
