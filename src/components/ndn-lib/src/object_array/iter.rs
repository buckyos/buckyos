use super::{storage::ObjectArrayInnerCache, ObjectArray};
use crate::{NdnResult, ObjId};

// Iterator over the object array, providing ObjId items
pub struct ObjectArrayIter<'a> {
    cache: &'a dyn ObjectArrayInnerCache,
    indices: std::ops::Range<usize>,
}

impl<'a> ObjectArrayIter<'a> {
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
        self.indices.size_hint()
    }
}

impl<'a> DoubleEndedIterator for ObjectArrayIter<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self.indices.next_back() {
            Some(index) => {
                match self.cache.get(index) {
                    Ok(Some(id)) => Some(id),
                    Ok(None) => {
                        // If the item is None, we just skip it
                        self.next_back() // Call next_back again to get the previous item
                    }
                    Err(_) => None,
                }
            }
            None => None,
        }
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

impl DoubleEndedIterator for ObjectArrayOwnedIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self.indices.next_back() {
            Some(index) => {
                match self.cache.get(index) {
                    Ok(Some(id)) => Some(id),
                    Ok(None) => {
                        // If the item is None, we just skip it
                        self.next_back() // Call next_back again to get the previous item
                    }
                    Err(_) => None,
                }
            }
            None => None,
        }
    }
}
