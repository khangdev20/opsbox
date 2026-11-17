//! The one-time token that proves you can read the server's console (spec 6.3).

/// 32 symbols, no `0 O 1 I`: easy to read from a console and copy by hand. 32 divides
/// 256, so masking a random byte is unbiased. Twelve symbols are 60 bits.
const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// `XXXX-XXXX-XXXX`
pub fn generate() -> Result<String, getrandom::Error> {
    let mut raw = [0u8; 12];
    getrandom::fill(&mut raw)?;
    let mut out = String::with_capacity(14);
    for (i, b) in raw.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        out.push(ALPHABET[(b & 31) as usize] as char);
    }
    Ok(out)
}

/// Canonical form for comparison: upper case, without spaces, dashes put back.
/// Returns `None` if it cannot possibly be a token.
pub fn normalize(input: &str) -> Option<String> {
    let compact: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_uppercase();
    if compact.len() != 12 || !compact.bytes().all(|b| ALPHABET.contains(&b)) {
        return None;
    }
    Some(format!(
        "{}-{}-{}",
        &compact[..4],
        &compact[4..8],
        &compact[8..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_have_the_documented_shape() {
        for _ in 0..50 {
            let t = generate().unwrap();
            assert_eq!(t.len(), 14);
            assert_eq!(normalize(&t).as_deref(), Some(t.as_str()), "{t}");
            assert!(!t.contains(['0', 'O', '1', 'I']));
        }
        assert_ne!(generate().unwrap(), generate().unwrap());
    }

    #[test]
    fn people_may_type_it_loosely() {
        assert_eq!(
            normalize(" 7qk2 9xmd-4tfa\n").as_deref(),
            Some("7QK2-9XMD-4TFA")
        );
        assert_eq!(normalize("7QK29XMD4TFA").as_deref(), Some("7QK2-9XMD-4TFA"));
    }

    #[test]
    fn impossible_tokens_are_rejected() {
        for bad in [
            "",
            "7QK2",
            "7QK2-9XMD-4TFA-X",
            "OOOO-1111-IIII",
            "7QK2-9XMD-4TF!",
        ] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
    }
}
