const MARKER: &str = "#buckyos-owner-v1=";

fn valid_marker(marker: &str) -> bool {
    marker.len() == 64
        && marker
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn scoped_owner_record_ref(raw: &str, marker: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    if !valid_marker(marker)
        || raw.len() > 4096
        || !matches!(url.scheme(), "https" | "obj" | "cyfs")
        || !url.username().is_empty()
        || url.password().is_some()
        || raw.chars().any(char::is_control)
    {
        return None;
    }
    Some(format!("{raw}{MARKER}{marker}"))
}

pub fn split_owner_record_ref(reference: &str) -> Option<(&str, &str)> {
    let (raw, marker) = reference.rsplit_once(MARKER)?;
    scoped_owner_record_ref(raw, marker)?;
    Some((raw, marker))
}
