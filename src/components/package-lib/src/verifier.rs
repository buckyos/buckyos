use crate::PkgResult;

pub struct Verifier {}

impl Verifier {
    pub async fn verify(author: &str, chunk_id: &str, sign: &str) -> PkgResult<()> {
        unimplemented!();
    }
}
