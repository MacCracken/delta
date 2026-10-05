//! Redaction of secret values from CI output.

/// Replacement text for a redacted secret.
const MASK: &str = "***";

/// Lines of multi-line secrets shorter than this are not redacted on their
/// own, so that e.g. a blank or one-character line doesn't mask all output.
const MIN_LINE_NEEDLE_LEN: usize = 4;

/// Build the list of values to redact from the given secret values.
///
/// Output is streamed line by line, so each line of a multi-line secret
/// (e.g. a PEM key) is also redacted on its own. Each value is matched
/// case-insensitively, both verbatim and in percent-encoded form.
pub fn secret_needles<'a>(secrets: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut needles = Vec::new();
    for secret in secrets {
        if secret.is_empty() {
            continue;
        }
        let mut push = |value: &str| {
            for candidate in [value.to_string(), percent_encode(value)] {
                if !needles.contains(&candidate) {
                    needles.push(candidate);
                }
            }
        };
        push(secret);
        if secret.contains('\n') {
            for line in secret.lines().map(str::trim) {
                if line.chars().count() >= MIN_LINE_NEEDLE_LEN {
                    push(line);
                }
            }
        }
    }
    // Longest first, so a secret is masked whole before any of its lines.
    needles.sort_by_key(|n| std::cmp::Reverse(n.len()));
    needles
}

/// Replace every case-insensitive occurrence of each needle in `text` with `***`.
pub fn mask_secrets(text: &str, needles: &[String]) -> String {
    let mut out = text.to_string();
    for needle in needles {
        if !needle.is_empty() {
            out = mask_one(&out, needle);
        }
    }
    out
}

fn mask_one(text: &str, needle: &str) -> String {
    let needle_lower: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some(len) = match_len(rest, &needle_lower) {
            out.push_str(MASK);
            rest = &rest[len..];
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// If `text` starts with the lowercased needle (comparing `text` lowercased
/// char by char), return the byte length of the matching prefix of `text`.
///
/// Matching walks the original string, so lowercase mappings that change
/// length (e.g. `İ`, `K`, `ẞ`) can never produce an offset that isn't a
/// char boundary of `text`.
fn match_len(text: &str, needle_lower: &[char]) -> Option<usize> {
    let mut matched = 0;
    for (idx, c) in text.char_indices() {
        for lc in c.to_lowercase() {
            if needle_lower.get(matched) != Some(&lc) {
                return None;
            }
            matched += 1;
        }
        if matched == needle_lower.len() {
            return Some(idx + c.len_utf8());
        }
    }
    None
}

/// Percent-encode everything except RFC 3986 unreserved characters.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(text: &str, secrets: &[&str]) -> String {
        mask_secrets(text, &secret_needles(secrets.iter().copied()))
    }

    #[test]
    fn masks_case_insensitively() {
        assert_eq!(mask("token=S3cret!", &["s3CRET!"]), "token=***");
        assert_eq!(mask("a s3cret b s3cret", &["s3cret"]), "a *** b ***");
    }

    #[test]
    fn masks_percent_encoded_form() {
        assert_eq!(mask("url?pw=p%40ss%2Fword", &["p@ss/word"]), "url?pw=***");
        assert_eq!(mask("url?pw=p%40ss%2fword", &["p@ss/word"]), "url?pw=***");
    }

    #[test]
    fn length_changing_lowercase_does_not_panic_or_shift() {
        // 'İ' lowercases to two chars ("i̇"), 'K' (Kelvin) to one ASCII byte.
        assert_eq!(mask("İx", &["x"]), "İ***");
        assert_eq!(mask("KKsecret", &["secret"]), "KK***");
        assert_eq!(mask("ẞ secret ẞ", &["secret"]), "ẞ *** ẞ");
    }

    #[test]
    fn masks_lines_of_multiline_secrets() {
        let key = "-----BEGIN KEY-----\nAAAABBBBCCCC\n-----END KEY-----";
        assert_eq!(mask("leaked: AAAABBBBCCCC", &[key]), "leaked: ***");
        assert_eq!(mask(&format!("dump {key}"), &[key]), "dump ***");
    }

    #[test]
    fn ignores_empty_secrets_and_short_lines() {
        assert_eq!(mask("hello", &[""]), "hello");
        assert_eq!(mask("a b c", &["xyz-long\nb"]), "a b c");
    }
}
