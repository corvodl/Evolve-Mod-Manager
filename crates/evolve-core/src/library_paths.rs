//! Safe, ordinary Windows path spellings at persistence and UI boundaries.
//! Keep canonical filesystem paths for containment checks; never treat a
//! lexical rewrite as proof that two existing filesystem objects are equal.

/// Remove only known Windows verbatim drive/UNC transport prefixes. Device
/// namespaces (such as \\.\ or \\?\Volume{...}) are deliberately untouched.
pub fn ordinary_path(value: &str) -> String {
    if let Some(tail) = value.strip_prefix(r"\\?\UNC\") {
        if !tail.is_empty() {
            return format!(r"\\{tail}");
        }
    }
    if let Some(tail) = value.strip_prefix(r"\\?\") {
        let bytes = tail.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            return tail.to_owned();
        }
    }
    value.to_owned()
}

/// Case/slash-insensitive comparison key for recorded Windows paths. This is
/// an identity hint, not authorization to read, write or delete a path.
pub fn source_path_key(value: &str) -> String {
    ordinary_path(value).replace('\\', "/").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_aliases_have_the_same_key() {
        assert_eq!(
            source_path_key("C:\\Games\\EvolveGame\\Game"),
            source_path_key(r"\\?\C:\Games\EvolveGame\Game")
        );
        assert_eq!(
            source_path_key(r"\\Server\Games\Evolve"),
            source_path_key(r"\\?\UNC\Server\Games\Evolve")
        );
    }

    #[test]
    fn only_known_prefixes_are_removed() {
        assert_eq!(ordinary_path(r"\\?\C:\Games"), r"C:\Games");
        assert_eq!(
            ordinary_path(r"\\?\UNC\server\share"),
            r"\\server\share"
        );
        assert_eq!(
            ordinary_path(r"\\?\Volume{123}\data"),
            r"\\?\Volume{123}\data"
        );
        assert_eq!(ordinary_path("/tmp/game"), "/tmp/game");
    }
}
