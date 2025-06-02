
use super::storage::ObjectArrayInnerCache;
use crate::{ObjId, NdnResult};


// Iterator over the object array, providing ObjId items
pub struct ObjectArrayIter<'a> {
    cache: &'a dyn ObjectArrayInnerCache,
    indices: std::ops::Range<usize>,
}

impl <'a> ObjectArrayIter<'a> {
    pub fn new(cache: &'a dyn ObjectArrayInnerCache) -> Self {
        let len = cache.len();
        Self {
            cache,
            indices: 0..len,
        }
    }
}

impl<'a> Iterator for ObjectArrayIter<'a> {
    type Item = ObjId;

    fn next(&mut self) -> Option<Self::Item> {
        match self.indices.next() {
            Some(index) => {
                match self.cache.get(index) {
                    Ok(Some(id)) => Some(id),
                    Ok(None) => {
                        // If the item is None, we just skip it
                        self.next() // Call next again to get the next item
                    }
                    Err(_) => {
                        // FIXME: What should we do on error? just return None now
                        None
                    }
                }
            }
            None => None,
        }
    }

    // Implement size_hint to help performance optimization
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.indices.end - self.indices.start;
        (remaining, Some(remaining))
    }
}

// Because we always known the size of the iterator, we can implement ExactSizeIterator
impl<'a> ExactSizeIterator for ObjectArrayIter<'a> {}

// This iterator consumes the ObjectArray and provides ObjId items
pub struct ObjectArrayOwnedIter {
    cache: Box<dyn ObjectArrayInnerCache>,
    indices: std::ops::Range<usize>,
}

impl ObjectArrayOwnedIter {
    pub fn new(cache: Box<dyn ObjectArrayInnerCache>) -> Self {
        let len = cache.len();
        Self {
            cache,
            indices: 0..len,
        }
    }
}

impl Iterator for ObjectArrayOwnedIter {
    type Item = ObjId;

    fn next(&mut self) -> Option<Self::Item> {
        match self.indices.next() {
            Some(index) => {
                match self.cache.get(index) {
                    Ok(Some(id)) => Some(id),
                    Ok(None) => {
                        // If the item is None, we just skip it
                        self.next() // Call next again to get the next item
                    }
                    Err(_) => {
                        // FIXME: What should we do on error? just return None now
                        None
                    }
                }
            }
            None => None,
        }
    }
}

impl ExactSizeIterator for ObjectArrayOwnedIter {}