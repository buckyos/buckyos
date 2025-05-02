// use reference_trie::ReferenceNodeCodec;
use trie_db::TrieLayout;
use sp_trie::LayoutV1;
use hash_db::Hasher;

/* 
// This is a placeholder for the ReferenceNodeCodec. 
// In a real implementation, you would import the actual codec from the reference_trie crate.
// But the reference_trie crate is only valid for testing, so we use a placeholder here.
pub struct ReferenceLayout<H: Hasher>(std::marker::PhantomData<H>);

impl<H: Hasher> TrieLayout for ReferenceLayout<H> {
    const USE_EXTENSION: bool = true;
    const ALLOW_EMPTY: bool = false;
    const MAX_INLINE_VALUE: Option<u32> = None;

    type Hash = H;
    type Codec = ReferenceNodeCodec<H>;
}
*/

pub type GenericLayout<H> = LayoutV1<H>;