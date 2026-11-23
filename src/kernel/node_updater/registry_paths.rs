//! Claims own their complete subtree. A star matches zero or more bytes within
//! a single path component, including when used in a filename suffix or prefix.
use std::collections::BTreeSet;

fn component_intersects(a: &[u8], b: &[u8]) -> bool {
    let mut next = vec![(0, 0)];
    let mut visited = BTreeSet::new();
    while let Some((i, j)) = next.pop() {
        if !visited.insert((i, j)) {
            continue;
        }
        match (a.get(i), b.get(j)) {
            (None, None) => return true,
            (Some(b'*'), _) => next.push((i + 1, j)),
            _ => {}
        }
        if b.get(j) == Some(&b'*') {
            next.push((i, j + 1));
        }
        if let (Some(&x), Some(&y)) = (a.get(i), b.get(j)) {
            if x == y || x == b'*' || y == b'*' {
                next.push((i + usize::from(x != b'*'), j + usize::from(y != b'*')));
            }
        }
    }
    false
}

pub fn overlaps(a: &str, b: &str) -> bool {
    a.split('/')
        .zip(b.split('/'))
        .all(|(x, y)| component_intersects(x.as_bytes(), y.as_bytes()))
}
