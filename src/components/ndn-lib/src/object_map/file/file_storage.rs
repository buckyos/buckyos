pub struct StoragePathGenerator {
}

impl StoragePathGenerator {
    pub fn gen_path(key: &str, name_len: usize, level: usize) -> String {
        assert!(name_len > 0);
        assert!(level > 0);

        let hash_str = base32::encode(
            base32::Alphabet::Rfc4648Lower { padding: false },
            &key.as_bytes(),
        );

        let mut path = String::new();
        let mut start = 0;
        for _ in 0..level {
            if start + name_len > hash_str.len() {
                break;
            }
            path.push_str(&hash_str[start..start + name_len]);
            path.push('/');
            start += name_len;
        }
        path.push_str(&key[start..]);
        path
    }
}
