//! The Zone's own domain (notepads/node-active-custom-domain-todo.md): what
//! its DNS records should be, whether they are there, and the credential and
//! certificate that go with it. The activation RPC (`active_server`) and the
//! RPC of an activated node (`runtime`) share [`service::DomainService`].

pub mod check;
pub mod dns;
pub mod plan;
pub mod remote;
pub mod runtime;
pub mod service;
#[cfg(test)]
pub mod testing;

/// A failure the pages translate by `code`; `detail` is never the main text.
#[derive(Clone, Debug, PartialEq)]
pub struct DomainError {
    pub code: String,
    pub detail: String,
}

impl DomainError {
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
        }
    }

    /// `[<scope>:<code>] <detail>`
    pub fn into_rpc_error(self, scope: &str) -> ::kRPC::RPCErrors {
        ::kRPC::RPCErrors::ReasonError(format!("[{scope}:{}] {}", self.code, self.detail))
    }
}

pub type DomainResult<T> = Result<T, DomainError>;
