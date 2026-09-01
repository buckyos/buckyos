pub(crate) const REDACTION_VERSION: u32 = 1;

const SECRET_KEYS: &[&str] = &[
    "session_token",
    "session-token",
    "refresh_token",
    "refresh-token",
    "access_token",
    "access-token",
    "private_key",
    "private-key",
    "client_secret",
    "client-secret",
    "api_key",
    "api-key",
    "apikey",
    "password",
    "passwd",
    "authorization",
    "token",
    "secret",
];

const DATABASE_SCHEMES: &[&str] = &[
    "postgres://",
    "postgresql://",
    "mysql://",
    "mariadb://",
    "mongodb://",
    "mongodb+srv://",
    "redis://",
    "rediss://",
    "sqlite://",
];

pub(crate) fn redact_text(value: &str) -> String {
    let value = redact_private_key_blocks(value);
    let value = redact_database_uris(&value);
    let value = redact_credential_urls(&value);
    let value = redact_jwts(&value);
    redact_named_secrets(&value)
}

fn redact_private_key_blocks(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    loop {
        let Some(begin) = rest.find("-----BEGIN ") else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..begin]);
        let candidate = &rest[begin..];
        let header_prefix_len = "-----BEGIN ".len();
        let Some(header_end) = candidate[header_prefix_len..]
            .find("-----")
            .map(|offset| header_prefix_len + offset)
        else {
            output.push_str(candidate);
            break;
        };
        let header = &candidate[..header_end + 5];
        if !header.contains("PRIVATE KEY") {
            output.push_str(&candidate[..header_end + 5]);
            rest = &candidate[header_end + 5..];
            continue;
        }
        let end_header = header.replacen("BEGIN", "END", 1);
        let Some(end) = candidate.find(&end_header) else {
            output.push_str("[REDACTED:private-key]");
            break;
        };
        output.push_str("[REDACTED:private-key]");
        rest = &candidate[end + end_header.len()..];
    }
    output
}

fn redact_database_uris(value: &str) -> String {
    redact_matching_tokens(value, |token| {
        let lower = token.to_ascii_lowercase();
        DATABASE_SCHEMES
            .iter()
            .any(|scheme| lower.starts_with(scheme))
            .then_some("[REDACTED:database-uri]")
    })
}

fn redact_credential_urls(value: &str) -> String {
    redact_matching_tokens(value, |token| {
        let scheme = token.find("://")?;
        let authority = &token[scheme + 3..];
        let at = authority.find('@')?;
        authority[..at]
            .contains(':')
            .then_some("[REDACTED:url-credentials]")
    })
}

fn redact_jwts(value: &str) -> String {
    redact_matching_tokens(value, |token| {
        let candidate = token.trim_matches(|ch: char| !is_token_char(ch));
        let mut parts = candidate.split('.');
        let header = parts.next()?;
        let payload = parts.next()?;
        let signature = parts.next()?;
        if parts.next().is_none()
            && header.starts_with("eyJ")
            && payload.len() >= 8
            && signature.len() >= 8
            && [header, payload, signature]
                .iter()
                .all(|part| part.chars().all(is_base64_url_char))
        {
            Some("[REDACTED:token]")
        } else {
            None
        }
    })
}

fn redact_matching_tokens(
    value: &str,
    replacement: impl Fn(&str) -> Option<&'static str>,
) -> String {
    let mut output = String::with_capacity(value.len());
    let mut start = 0;
    for (index, character) in value.char_indices() {
        if character.is_whitespace() || matches!(character, '"' | '\'' | '<' | '>' | ',' | ';') {
            if start < index {
                let token = &value[start..index];
                output.push_str(replacement(token).unwrap_or(token));
            }
            output.push(character);
            start = index + character.len_utf8();
        }
    }
    if start < value.len() {
        let token = &value[start..];
        output.push_str(replacement(token).unwrap_or(token));
    }
    output
}

fn redact_named_secrets(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while cursor < value.len() {
        let next = SECRET_KEYS
            .iter()
            .filter_map(|key| {
                lower[cursor..]
                    .find(key)
                    .map(|offset| (cursor + offset, *key))
            })
            .min_by_key(|(index, _)| *index);
        let Some((index, key)) = next else {
            output.push_str(&value[cursor..]);
            break;
        };
        let before_is_word = index
            .checked_sub(1)
            .and_then(|previous| value.as_bytes().get(previous))
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
        let key_end = index + key.len();
        let after_is_word = value
            .as_bytes()
            .get(key_end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
        if before_is_word || after_is_word {
            output.push_str(&value[cursor..key_end]);
            cursor = key_end;
            continue;
        }

        let mut separator = key_end;
        if value
            .as_bytes()
            .get(separator)
            .is_some_and(|byte| *byte == b'"')
        {
            separator += 1;
        }
        while value
            .as_bytes()
            .get(separator)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            separator += 1;
        }
        if !value
            .as_bytes()
            .get(separator)
            .is_some_and(|byte| matches!(*byte, b'=' | b':'))
        {
            output.push_str(&value[cursor..key_end]);
            cursor = key_end;
            continue;
        }
        separator += 1;
        while value
            .as_bytes()
            .get(separator)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            separator += 1;
        }
        let quote = value
            .as_bytes()
            .get(separator)
            .copied()
            .filter(|byte| matches!(*byte, b'"' | b'\''));
        let value_start = separator + usize::from(quote.is_some());
        let mut value_end = value_start;
        while let Some(byte) = value.as_bytes().get(value_end) {
            let done = match quote {
                Some(quote) => *byte == quote,
                None if key == "authorization" => {
                    matches!(*byte, b'\n' | b'\r' | b',' | b';' | b'}' | b']')
                }
                None => byte.is_ascii_whitespace() || matches!(*byte, b',' | b';' | b'}' | b']'),
            };
            if done {
                break;
            }
            value_end += 1;
        }
        output.push_str(&value[cursor..value_start]);
        output.push_str("[REDACTED:secret]");
        cursor = value_end;
    }
    output
}

fn is_token_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
}

fn is_base64_url_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_required_secret_classes() {
        let input = concat!(
            "password=hunter2 session_token: eyJheader12345.eyJpayload12345.signature12345 ",
            "postgres://alice:pw@db.internal/app ",
            "https://alice:pw@example.test/path ",
            "api_key='external-secret'\n",
            "Authorization: Bearer third-party-token\n",
            "-----BEGIN PRIVATE KEY-----\nprivate-material\n-----END PRIVATE KEY-----",
        );
        let output = redact_text(input);
        for secret in [
            "hunter2",
            "eyJpayload12345",
            "alice:pw",
            "external-secret",
            "third-party-token",
            "private-material",
        ] {
            assert!(!output.contains(secret), "secret leaked: {secret}");
        }
        assert!(output.contains("[REDACTED:database-uri]"));
        assert!(output.contains("[REDACTED:private-key]"));
    }

    #[test]
    fn leaves_normal_log_text_unchanged() {
        let input = "08-25 10:00:00.000 [INFO] scheduler started task t-123";
        assert_eq!(redact_text(input), input);
    }
}
