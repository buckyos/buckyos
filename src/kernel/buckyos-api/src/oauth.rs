use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthSourceKind {
    Google,
    Oidc,
    #[serde(rename = "oauth2")]
    OAuth2,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthSource {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub kind: OAuthSourceKind,
    pub client_id: String,
    pub secret_ref: String,
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub webhost: String,
    #[serde(default)]
    pub issuer: String,
    #[serde(default)]
    pub authorization_endpoint: String,
    #[serde(default)]
    pub token_endpoint: String,
    #[serde(default)]
    pub jwks_uri: String,
    #[serde(default)]
    pub account_endpoint: String,
    #[serde(default)]
    pub id_field: String,
    #[serde(default)]
    pub email_field: String,
    #[serde(default)]
    pub email_verified_field: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicOAuthSource {
    pub id: String,
    pub display_name: String,
}

pub const OAUTH_SUBJECTS_PREFIX: &str = "services/control_panel/oauth/subjects";
/// Deleted login sources. Sessions, grants and account bindings name a source by
/// its ID, identity domain and issuer, so a deleted source's namespace stays
/// reserved: only the same source can be added again, and then its existing
/// bindings sign in again.
pub const RETIRED_OAUTH_SOURCES_KEY: &str = "services/control_panel/oauth/retired_sources";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetiredOAuthSource {
    pub id: String,
    pub kind: OAuthSourceKind,
    pub webhost: String,
    pub issuer: String,
}

impl RetiredOAuthSource {
    pub fn of(source: &OAuthSource) -> Self {
        Self {
            id: source.id.clone(),
            kind: source.kind,
            webhost: source.webhost.clone(),
            issuer: source.issuer.clone(),
        }
    }

    pub fn claims(&self, source: &OAuthSource) -> bool {
        self.id == source.id || self.webhost == source.webhost || self.issuer == source.issuer
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifiedOAuth {
    pub source_id: String,
    pub issuer: String,
    pub sub: String,
    pub email: String,
    pub label: String,
    pub did: String,
    pub webhost: String,
}

/// `services/control_panel/oauth/subjects/<oauth_subject_key>`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuthSubject {
    pub identity: VerifiedOAuth,
    pub did: String,
    pub email: String,
    #[serde(skip)]
    pub bound_user: Option<String>,
}

pub fn oauth_subject_key(issuer: &str, sub: &str) -> String {
    let hash = Sha256::digest(serde_json::to_vec(&(issuer, sub)).expect("strings serialize"));
    format!("{hash:x}")
}

pub fn oauth_label_key(did: &str) -> String {
    format!(
        "services/control_panel/oauth/labels/{:x}",
        Sha256::digest(did.as_bytes())
    )
}

pub fn oauth_binding_key(issuer: &str, sub: &str) -> String {
    format!(
        "services/control_panel/oauth/bindings/{}",
        oauth_subject_key(issuer, sub)
    )
}

pub fn valid_oauth_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
}

fn dns_domain(domain: &str) -> bool {
    domain.len() <= 253
        && domain.contains('.')
        && domain.parse::<std::net::IpAddr>().is_err()
        && domain.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        })
}

fn at_or_under(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn https(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "invalid OAuth endpoint")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(
            "OAuth endpoints must be HTTPS URLs without credentials, query or fragment".into(),
        );
    }
    Ok(url)
}

pub fn validate_oauth_sources(sources: &mut [OAuthSource], zone: &str) -> Result<(), String> {
    let mut ids = HashSet::new();
    let mut domains = HashSet::new();
    let mut issuers = HashSet::new();
    for source in sources {
        if !valid_oauth_id(&source.id)
            || !ids.insert(source.id.clone())
            || source.display_name.trim().is_empty()
            || source.client_id.trim().is_empty()
            || !valid_oauth_id(&source.secret_ref)
        {
            return Err(
                "invalid or duplicate OAuth source ID, display name, client ID or secret reference"
                    .into(),
            );
        }
        let callback = format!("https://{zone}/sso/oauth/{}/callback", source.id);
        if !source.redirect_uri.is_empty() && source.redirect_uri != callback {
            return Err(
                "registered OAuth redirect URI must equal the HTTPS callback on this Zone".into(),
            );
        }
        source.redirect_uri = callback;
        match source.kind {
            OAuthSourceKind::Google => {
                if source.id != "google" {
                    return Err("the Google preset uses source ID google".into());
                }
                source.webhost = "google.com".into();
                source.issuer = "https://accounts.google.com".into();
                source.authorization_endpoint =
                    "https://accounts.google.com/o/oauth2/v2/auth".into();
                source.token_endpoint = "https://oauth2.googleapis.com/token".into();
                source.jwks_uri = "https://www.googleapis.com/oauth2/v3/certs".into();
                source.scopes = vec!["openid".into(), "email".into()];
            }
            OAuthSourceKind::Oidc | OAuthSourceKind::OAuth2 => {
                source.webhost = source.webhost.trim().to_ascii_lowercase();
                if source.id == "google" || at_or_under(&source.webhost, "google.com") {
                    return Err("Google identity namespace is reserved".into());
                }
                let authority = if source.kind == OAuthSourceKind::Oidc {
                    https(&source.jwks_uri)?;
                    for scope in ["openid", "email"] {
                        if !source.scopes.iter().any(|s| s == scope) {
                            source.scopes.push(scope.into());
                        }
                    }
                    https(&source.issuer)?
                } else {
                    if [
                        &source.id_field,
                        &source.email_field,
                        &source.email_verified_field,
                    ]
                    .iter()
                    .any(|field| field.trim().is_empty())
                        || source.id_field == source.email_field
                        || source.email_field == source.email_verified_field
                        || source.id_field == source.email_verified_field
                    {
                        return Err("OAuth account endpoint needs separate stable ID, email and verified-email fields".into());
                    }
                    source.issuer = source.account_endpoint.clone();
                    https(&source.account_endpoint)?
                };
                if !at_or_under(authority.host_str().unwrap_or_default(), &source.webhost) {
                    return Err("identity domain must contain the provider authority".into());
                }
                https(&source.authorization_endpoint)?;
                https(&source.token_endpoint)?;
            }
        }
        if !dns_domain(&source.webhost)
            || at_or_under(&source.webhost, zone)
            || !domains.insert(source.webhost.clone())
            || !issuers.insert(source.issuer.clone())
        {
            return Err(
                "invalid, overlapping Zone, or duplicate OAuth identity domain or issuer".into(),
            );
        }
    }
    Ok(())
}

pub fn oauth_account_label(google: bool, email: &str) -> Result<(String, String), String> {
    let address = email.trim().to_lowercase();
    let (user, domain) = address.split_once('@').ok_or("invalid email")?;
    if address.len() > 254
        || user.is_empty()
        || user.len() > 64
        || address.chars().any(|c| c.is_whitespace() || c.is_control())
        || !dns_domain(domain)
    {
        return Err("invalid email".into());
    }
    if google && matches!(domain, "gmail.com" | "googlemail.com") {
        let user = user.split('+').next().unwrap_or_default().replace('.', "");
        if user.is_empty()
            || user.len() > 63
            || !user
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return Err("invalid Gmail username".into());
        }
        return Ok((format!("{user}@gmail.com"), user));
    }
    const BASE32: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut label = String::from("e-");
    let digest = Sha256::digest(address.as_bytes());
    for chunk in digest[..20].chunks_exact(5) {
        let word = u64::from_be_bytes([0, 0, 0, chunk[0], chunk[1], chunk[2], chunk[3], chunk[4]]);
        for i in (0..8).rev() {
            label.push(BASE32[((word >> (i * 5)) & 31) as usize] as char);
        }
    }
    Ok((address, label))
}

pub fn valid_oauth_label(google: bool, label: &str) -> bool {
    label.strip_prefix("e-").is_some_and(|v| {
        v.len() == 32
            && v.bytes()
                .all(|c| c.is_ascii_lowercase() || (b'2'..=b'7').contains(&c))
    }) || google
        && (1..=63).contains(&label.len())
        && label
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

pub fn external_source_for_did<'a>(settings: &'a crate::SsoSettings, did: &str) -> Option<&'a str> {
    let id = did.strip_prefix("did:web:")?;
    let host = id
        .split([':', '/', '#', '?', '%'])
        .next()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if at_or_under(&host, "google.com") {
        return Some("google");
    }
    settings
        .oauth_sources
        .iter()
        .filter(|source| {
            at_or_under(&host, &source.webhost)
                && !Url::parse(&source.redirect_uri)
                    .ok()
                    .is_some_and(|url| url.host_str().is_some_and(|zone| at_or_under(&host, zone)))
        })
        .max_by_key(|s| s.webhost.len())
        .map(|s| s.id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn email_labels() {
        for (input, address, label) in [
            (" Alice@gmail.com ", "alice@gmail.com", "alice"),
            ("a.l.i.c.e+work@googlemail.com", "alice@gmail.com", "alice"),
            (
                "Alice@Company.com",
                "alice@company.com",
                "e-elezio5g7xybatgcdn5urmolneorulja",
            ),
            (
                "bob.smith@company.com",
                "bob.smith@company.com",
                "e-lfyluyqyaq2ucyjbqnol6xwg6rc4s3vf",
            ),
        ] {
            assert_eq!(
                oauth_account_label(true, input).unwrap(),
                (address.into(), label.into())
            );
        }
        assert!(oauth_account_label(false, "a.l.i.c.e+work@googlemail.com")
            .unwrap()
            .1
            .starts_with("e-"));
        assert!(oauth_account_label(true, "a-b@gmail.com").is_err());
        assert_eq!(
            external_source_for_did(&Default::default(), "did:web:alice.google.com"),
            Some("google")
        );
        assert_eq!(
            external_source_for_did(&Default::default(), "did:web:alice.notgoogle.com"),
            None
        );
    }
}
