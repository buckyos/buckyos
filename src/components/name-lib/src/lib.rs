#![allow(dead_code)]
#![allow(unused)]

mod utility;
mod provider;
mod name_client;
mod name_query;
mod dns_provider;
mod did;


pub use did::*;
pub use provider::*;
pub use utility::*;
pub use name_client::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_utility() {
        assert_eq!(is_did("did:example:123456789abcdefghi"), true);
        assert_eq!(is_did("www.buckyos.org"), false);
    }

}
