//! Credentials for the worker, and the browser sign-in for OAuth accounts.
//!
//! The keychain holds, under the account id, the password (`Auth::Password`) or the refresh
//! token (`Auth::OAuth2`); a Google client secret sits under `<id>#client_secret`. Access
//! tokens are short-lived and only ever live in the worker's memory.

use std::sync::Arc;
use std::time::{Duration, Instant};

use merel_model::{Account, Auth, Protocol};
use merel_oauth::{Client, OAuthError};
use merel_secrets::{Secret, SecretStore};

/// The keychain key of an account's client secret.
pub fn client_secret_key(account_id: &str) -> String {
    format!("{account_id}#client_secret")
}

pub fn oauth_client(account: &Account, client_secret: Option<Secret>) -> Option<Client> {
    match &account.auth {
        Auth::Password => None,
        Auth::OAuth2 { client_id, auth_url, token_url, scope } => Some(Client {
            client_id: client_id.clone(),
            client_secret,
            auth_url: auth_url.clone(),
            token_url: token_url.clone(),
            scope: scope.clone(),
        }),
    }
}

/// What the worker holds to log in: a password, or a refresh token with a cached access token.
pub struct Credentials {
    secret: Secret,
    client: Option<Client>,
    access: Option<(Secret, Instant)>,
}

impl Credentials {
    pub fn new(account: &Account, secret: Secret, client_secret: Option<Secret>) -> Self {
        Credentials { secret, client: oauth_client(account, client_secret), access: None }
    }

    /// The credential to log in with right now, refreshing the access token when needed.
    pub fn incoming(&mut self) -> Result<merel_mailbox::Credential, String> {
        Ok(match self.access_token()? {
            Some(t) => merel_mailbox::Credential::OAuth2(t),
            None => merel_mailbox::Credential::Password(self.secret.clone()),
        })
    }

    pub fn smtp(&mut self) -> Result<merel_smtp::Credential, String> {
        Ok(match self.access_token()? {
            Some(t) => merel_smtp::Credential::OAuth2(t),
            None => merel_smtp::Credential::Password(self.secret.clone()),
        })
    }

    /// Drops the cached access token, so the next login refreshes it.
    pub fn invalidate(&mut self) {
        self.access = None;
    }

    fn access_token(&mut self) -> Result<Option<Secret>, String> {
        let Some(client) = &self.client else { return Ok(None) };
        if let Some((t, until)) = &self.access
            && Instant::now() < *until
        {
            return Ok(Some(t.clone()));
        }
        let tokens = client.refresh(&self.secret).map_err(|e| describe(&e))?;
        let until = Instant::now() + tokens.expires_in.saturating_sub(Duration::from_secs(60));
        self.access = Some((tokens.access_token.clone(), until));
        Ok(Some(tokens.access_token))
    }
}

pub fn describe(e: &OAuthError) -> String {
    match e {
        OAuthError::Listen(_) => "Kon geen poort openen voor het aanmelden.".into(),
        OAuthError::Browser(_) => "Kon de browser niet openen.".into(),
        OAuthError::Cancelled => "Het aanmelden is afgebroken of duurde te lang.".into(),
        OAuthError::State => "Het aanmelden kwam verkeerd terug; probeer het opnieuw.".into(),
        OAuthError::Provider(m) => format!("De aanbieder weigerde: {}", merel_model::clean_text(m)),
        OAuthError::Http(_) => "Kon de aanbieder niet bereiken.".into(),
    }
}

/// What the Backstage hands over to start a browser sign-in.
#[derive(Clone, Debug)]
pub struct SignIn {
    pub account: Account,
    pub client_secret: Option<Secret>,
}

/// Runs the browser sign-in on the calling thread and stores what it yields.
pub fn run(secrets: &Arc<dyn SecretStore>, sign_in: SignIn) -> Result<Account, String> {
    let Some(client) = oauth_client(&sign_in.account, sign_in.client_secret.clone()) else {
        return Err("Dit account gebruikt geen aanmelding via de browser.".into());
    };
    let tokens = client.sign_in(true).map_err(|e| describe(&e))?;
    let refresh = tokens
        .refresh_token
        .ok_or_else(|| "De aanbieder gaf geen vernieuwingstoken; probeer opnieuw en geef toestemming voor offline toegang.".to_string())?;
    secrets.set(&sign_in.account.id, &refresh).map_err(|e| e.to_string())?;
    if let Some(cs) = &sign_in.client_secret {
        secrets.set(&client_secret_key(&sign_in.account.id), cs).map_err(|e| e.to_string())?;
    }
    Ok(sign_in.account)
}

/// A new OAuth account from the form fields, with the provider's preset servers. An empty
/// client ID falls back to the one built into Merel, where there is one.
pub fn account(protocol: Protocol, name: &str, email: &str, client_id: &str) -> Result<Account, String> {
    account_with(protocol, name, email, client_id, protocol.builtin_client_id())
}

fn account_with(protocol: Protocol, name: &str, email: &str, client_id: &str, builtin: Option<&str>) -> Result<Account, String> {
    let preset = protocol.preset().ok_or_else(|| "Kies Gmail of Microsoft 365.".to_string())?;
    let address = merel_model::Address::new(Some(name.trim()), email.trim()).map_err(|_| "Vul een geldig e-mailadres in.".to_string())?;
    let client_id = match client_id.trim() {
        "" => builtin.unwrap_or_default(),
        own => own,
    };
    if client_id.is_empty() || client_id.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("Vul de client-ID in die je bij de aanbieder hebt aangemaakt.".into());
    }
    let id = format!(
        "{}-{}",
        address.email().replace(['@', '.'], "-"),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    );
    Ok(Account {
        id,
        name: if name.trim().is_empty() { address.email().to_string() } else { name.trim().to_string() },
        username: address.email().to_string(),
        address,
        protocol,
        auth: Auth::OAuth2 { client_id: client_id.to_string(), auth_url: preset.auth_url, token_url: preset.token_url, scope: preset.scope },
        incoming: Some(preset.incoming),
        outgoing: preset.outgoing,
        dav: None,
        signature: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_account_takes_the_preset() {
        let a = account(Protocol::Gmail, "Alain", "alain@gmail.com", "abc.apps.googleusercontent.com").unwrap();
        assert_eq!(a.incoming.unwrap().host, "imap.gmail.com");
        assert!(matches!(a.auth, Auth::OAuth2 { ref scope, .. } if scope.starts_with("https://mail.google.com/ ") && scope.contains("/auth/tasks")));
        assert!(account(Protocol::ImapSmtp, "", "a@b.nl", "x").is_err());
        assert!(account(Protocol::Gmail, "", "a@b.nl", " ").is_err());
    }

    #[test]
    fn an_empty_client_id_takes_the_builtin_one() {
        let builtin = account_with(Protocol::Microsoft365, "", "a@outlook.com", " ", Some("built-in"));
        assert!(matches!(builtin.unwrap().auth, Auth::OAuth2 { ref client_id, .. } if client_id == "built-in"));
        let own = account_with(Protocol::Microsoft365, "", "a@outlook.com", "own", Some("built-in"));
        assert!(matches!(own.unwrap().auth, Auth::OAuth2 { ref client_id, .. } if client_id == "own"));
        assert!(account_with(Protocol::Microsoft365, "", "a@outlook.com", "", None).is_err());
        assert_eq!(Protocol::Gmail.builtin_client_id(), None);
        assert!(Protocol::Microsoft365.builtin_client_id().is_some());
    }

    #[test]
    fn password_accounts_never_refresh() {
        let a = account(Protocol::Gmail, "", "a@gmail.com", "id").map(|mut a| {
            a.auth = Auth::Password;
            a
        });
        let mut c = Credentials::new(&a.unwrap(), Secret::new("pw"), None);
        assert!(matches!(c.incoming().unwrap(), merel_mailbox::Credential::Password(_)));
        assert!(matches!(c.smtp().unwrap(), merel_smtp::Credential::Password(_)));
    }
}
