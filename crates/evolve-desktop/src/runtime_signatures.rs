//! Bounded, read-only address signature helpers for private Evolve runtime
//! research. Ports the private Python manager's AOB matching semantics.
//! Does not scan arbitrary processes or patch executable code.
const MAX_WINDOW: usize = 256 * 1024;
const MAX_PATTERN: usize = 128;
const MAX_RESULTS: usize = 256;

pub(super) fn parse_aob(input: &str) -> Result<Vec<Option<u8>>, String> {
    let words = input.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() || words.len() > MAX_PATTERN {
        return Err("Use 1 to 128 signature bytes; ?? is a wildcard".into());
    }
    let mut signature = Vec::with_capacity(words.len());
    for word in words {
        if word == "?" || word == "??" {
            signature.push(None);
            continue;
        }
        if word.len() != 2 || !word.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("Invalid signature byte: {word}"));
        }
        let byte = u8::from_str_radix(word, 16).map_err(|_| "Invalid signature byte")?;
        signature.push(Some(byte));
    }
    if !signature.iter().any(Option::is_some) {
        return Err("Signature requires at least one fixed byte".into());
    }
    Ok(signature)
}

pub(super) fn find_aob(
    bytes: &[u8],
    base: usize,
    signature: &[Option<u8>],
) -> Result<Vec<usize>, String> {
    if bytes.len() > MAX_WINDOW {
        return Err("Signature search cannot exceed 256 KiB".into());
    }
    if signature.is_empty()
        || signature.len() > MAX_PATTERN
        || signature.iter().all(Option::is_none)
    {
        return Err("Invalid signature".into());
    }
    if signature.len() > bytes.len() {
        return Ok(Vec::new());
    }
    let mut hits = Vec::new();
    for i in 0..=bytes.len() - signature.len() {
        if signature
            .iter()
            .enumerate()
            .all(|(j, expected)| expected.is_none_or(|v| bytes[i + j] == v))
        {
            let at = base.checked_add(i).ok_or("Address overflow")?;
            hits.push(at);
            if hits.len() == MAX_RESULTS {
                break;
            }
        }
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_and_overlapping_matches_are_supported() {
        let pattern = parse_aob("AA ?? AA").unwrap();
        let found = find_aob(&[0xAA, 0xAA, 0xAA, 0xAA], 0x1000, &pattern).unwrap();
        assert_eq!(found, vec![0x1000, 0x1001]);
    }

    #[test]
    fn signatures_are_bounded_and_unambiguous() {
        assert!(parse_aob("?? ??").is_err());
        assert!(parse_aob("5").is_err());
        assert!(parse_aob("HH").is_err());
        assert!(parse_aob(&["AA"; 129].join(" ")).is_err());
        assert!(find_aob(&vec![0; 256 * 1024 + 1], 0, &[Some(0)]).is_err());
        assert!(find_aob(&[], 0, &[]).is_err());
    }

    #[test]
    fn result_count_is_limited_and_address_overflow_fails_closed() {
        let pattern = parse_aob("00").unwrap();
        assert_eq!(
            find_aob(&vec![0; 400], 0x1000, &pattern).unwrap().len(),
            256
        );
        assert!(find_aob(&[0, 0], usize::MAX, &pattern).is_err());
    }
}
