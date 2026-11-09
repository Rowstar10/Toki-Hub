//! Ordering version strings the way a person expects: 1.10 > 1.9 > 1.0 > 1.0-beta.

/// Sort key for a version string; compare in reverse for newest first.
///
/// Numbers compare as numbers, "1.0" equals "1.0.0", a release sorts after its own pre-releases
/// ("1.0" > "1.0-rc2" > "1.0-beta"), and versions that do not start with a number sort below all
/// numbered ones, alphabetically.
pub fn version_key(version: &str) -> (Vec<u64>, u8, String) {
    let text = version.trim();
    let body = text.strip_prefix(['v', 'V']).unwrap_or(text);
    let bytes = body.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_digit) {
        return (Vec::new(), 0, text.to_lowercase());
    }
    let mut numbers = Vec::new();
    let mut i = 0;
    loop {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        numbers.push(body[start..i].parse::<u64>().unwrap_or(u64::MAX));
        if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
            i += 1;
        } else {
            break;
        }
    }
    while numbers.len() > 1 && numbers.last() == Some(&0) {
        numbers.pop();
    }
    let suffix = body[i..].trim_matches(|c: char| " -_.+".contains(c)).to_lowercase();
    let release = if suffix.is_empty() { 1 } else { 0 };
    (numbers, release, suffix)
}

pub fn sort_newest_first<T>(items: &mut [T], name: impl Fn(&T) -> &str) {
    items.sort_by(|a, b| version_key(name(b)).cmp(&version_key(name(a))));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first() {
        let mut versions = vec!["1.0", "1.10", "1.9", "2.0-beta", "2.0", "0.9", "nightly", "2.0-rc1"];
        sort_newest_first(&mut versions, |v| v);
        assert_eq!(versions, ["2.0", "2.0-rc1", "2.0-beta", "1.10", "1.9", "1.0", "0.9", "nightly"]);
    }

    #[test]
    fn equivalent_spellings() {
        assert_eq!(version_key("1.0"), version_key("1.0.0"));
        assert_eq!(version_key("v1.2"), version_key("1.2"));
    }
}
