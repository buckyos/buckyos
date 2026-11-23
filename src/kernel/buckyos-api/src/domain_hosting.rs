//! How the Zone's own domain is hosted: who manages its DNS and where its
//! certificate comes from. Written by activation into `start_config.json`
//! (`domain_hosting`), kept in system-config at [`DOMAIN_HOSTING_KEY`]. A Zone
//! on the default `<user>.web3.<SN>` name has none.

use serde::{Deserialize, Serialize};

pub const DOMAIN_HOSTING_KEY: &str = "system/domain_hosting";
pub const START_CONFIG_DOMAIN_HOSTING: &str = "domain_hosting";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DomainHostingMode {
    /// NS delegated to the SN, which answers every record.
    Sn,
    /// The user writes the records.
    Manual,
    /// The node writes the records with the user's token.
    DnsApi,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DomainCertificateSource {
    /// Issued and renewed by the system.
    Acme,
    /// Uploaded; never renewed, replaced or watched by the system.
    Upload,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DomainHosting {
    pub mode: DomainHostingMode,
    pub certificate: DomainCertificateSource,
    pub domain: String,
    /// DNS service id, `dns_api` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// File name of the token under `$BUCKYOS_ROOT/security/dns_credentials/`, `dns_api` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    /// ACME directory used instead of Let's Encrypt production (DV against staging).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acme_issuer: Option<String>,
}

impl DomainHosting {
    pub fn validate(&self) -> Result<(), String> {
        if self.domain.is_empty() || self.domain != self.domain.trim().to_ascii_lowercase() {
            return Err(format!("invalid domain `{}`", self.domain));
        }
        if self.mode == DomainHostingMode::Manual
            && self.certificate != DomainCertificateSource::Upload
        {
            return Err("manual DNS needs an uploaded certificate".to_string());
        }
        let has_credential = self.provider.is_some() && self.credential_id.is_some();
        if (self.mode == DomainHostingMode::DnsApi) != has_credential
            || self.provider.is_some() != self.credential_id.is_some()
        {
            return Err("provider and credential_id belong to dns_api, and only to it".to_string());
        }
        Ok(())
    }

    pub fn uses_acme(&self) -> bool {
        self.certificate == DomainCertificateSource::Acme
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: serde_json::Value) -> Result<DomainHosting, String> {
        let hosting: DomainHosting =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        hosting.validate()?;
        Ok(hosting)
    }

    #[test]
    fn only_consistent_hosting_is_accepted() {
        for value in [
            json!({"mode":"sn","certificate":"acme","domain":"home.example.com"}),
            json!({"mode":"sn","certificate":"upload","domain":"home.example.com"}),
            json!({"mode":"manual","certificate":"upload","domain":"example.com"}),
            json!({"mode":"dns_api","certificate":"acme","domain":"home.example.com",
                   "provider":"cloudflare","credential_id":"home-example-com"}),
            json!({"mode":"sn","certificate":"acme","domain":"home.example.com",
                   "acme_issuer":"https://acme-staging-v02.api.letsencrypt.org/directory"}),
        ] {
            parse(value.clone()).unwrap_or_else(|error| panic!("{value}: {error}"));
        }
        for value in [
            json!({"mode":"manual","certificate":"acme","domain":"home.example.com"}),
            json!({"mode":"dns_api","certificate":"acme","domain":"home.example.com"}),
            json!({"mode":"dns_api","certificate":"acme","domain":"home.example.com","provider":"cloudflare"}),
            json!({"mode":"sn","certificate":"acme","domain":"home.example.com","provider":"cloudflare","credential_id":"x"}),
            json!({"mode":"sn","certificate":"acme","domain":"Home.example.com"}),
            json!({"mode":"sn","certificate":"acme","domain":"home.example.com","token":"x"}),
        ] {
            assert!(parse(value.clone()).is_err(), "{value}");
        }
    }
}
