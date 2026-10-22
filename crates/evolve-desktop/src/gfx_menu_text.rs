//! Safe, preview-only localization of original SWF DefineEditText INITIAL values.
//! This does not create replacement menu buttons, translate arbitrary scripts,
//! or edit source GFX. Runtime-created TextFields still require CryEngine host
//! callbacks and are deliberately reported as outside this patch's scope.
use evolve_core::gfx::MAX_DECOMPRESSED;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Default, Debug, PartialEq, Eq)]
pub struct TextPatch {
    pub patched: usize,
    pub unresolved: usize,
    pub original_edit_fields: usize,
}

fn label(text: &str) -> Option<String> {
    let raw = text.trim();
    let key = raw.strip_prefix('@')?;
    if !(3..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
    {
        return None;
    }
    Some(key.to_ascii_lowercase())
}
fn zstring_end(data: &[u8], at: usize) -> Option<usize> {
    let rest = data.get(at..)?;
    Some(at + rest.iter().position(|&c| c == 0)?)
}
fn skip(data: &[u8], cursor: &mut usize, amount: usize) -> Option<()> {
    *cursor = cursor.checked_add(amount)?;
    (*cursor <= data.len()).then_some(())
}
/// The body of SWF DefineEditText is id, RECT, 16 packed option bits, optional
/// Font/FontClass, color, max length, layout, variable name, initial text.
/// Preserve every original field byte; only the zero-terminated initial text
/// may be replaced with a verified exact-build translation.
fn edit_text_value(tag: &[u8]) -> Option<(usize, usize)> {
    if tag.len() < 6 {
        return None;
    }
    let bits = usize::from(tag[2] >> 3);
    if !(1..=31).contains(&bits) {
        return None;
    }
    let mut at = 2 + (5 + 4 * bits).div_ceil(8);
    if at + 2 > tag.len() {
        return None;
    }
    let flags = u16::from_be_bytes([tag[at], tag[at + 1]]);
    at += 2;
    if flags & 0x0100 != 0 {
        skip(tag, &mut at, 2)?;
    }
    if flags & 0x0080 != 0 {
        at = zstring_end(tag, at)? + 1;
    }
    if flags & 0x0100 != 0 {
        skip(tag, &mut at, 2)?;
    }
    if flags & 0x0400 != 0 {
        skip(tag, &mut at, 4)?;
    }
    if flags & 0x0200 != 0 {
        skip(tag, &mut at, 2)?;
    }
    if flags & 0x0020 != 0 {
        skip(tag, &mut at, 9)?;
    }
    at = zstring_end(tag, at)? + 1; // authored variable name
    if flags & 0x8000 == 0 {
        return None;
    }
    let end = zstring_end(tag, at)?;
    // Never modify malformed tags or anything beyond the initial string.
    (end + 1 == tag.len()).then_some((at, end))
}

fn walk<F>(swf: &[u8], mut tag_fn: F) -> Result<Vec<u8>, String>
where
    F: FnMut(u16, &[u8]) -> Result<Vec<u8>, String>,
{
    if swf.len() < 20 || swf.len() > MAX_DECOMPRESSED || &swf[..3] != b"FWS" {
        return Err("Original menu localization requires a validated uncompressed SWF".into());
    }
    let declared = u32::from_le_bytes(swf[4..8].try_into().unwrap()) as usize;
    if declared != swf.len() {
        return Err("SWF length does not match its header".into());
    }
    let nbits = usize::from(swf[8] >> 3);
    if !(1..=31).contains(&nbits) {
        return Err("Original main-menu SWF RECT is malformed".into());
    }
    let start = 8 + (5 + 4 * nbits).div_ceil(8) + 4;
    if start > swf.len() {
        return Err("Original main-menu SWF frame header is truncated".into());
    }
    let mut dest = swf[..start].to_vec();
    let mut at = start;
    let mut count = 0usize;
    while at + 2 <= swf.len() && count < 80_000 {
        let tag_begin = at;
        let header = u16::from_le_bytes([swf[at], swf[at + 1]]);
        at += 2;
        let kind = header >> 6;
        let mut length = usize::from(header & 63);
        if length == 63 {
            if at + 4 > swf.len() {
                return Err("Truncated extended SWF tag".into());
            }
            length = u32::from_le_bytes(swf[at..at + 4].try_into().unwrap()) as usize;
            at += 4;
        }
        if length > swf.len() - at {
            return Err("SWF contains an invalid tag length".into());
        }
        let updated = tag_fn(kind, &swf[at..at + length])?;
        if updated.len()
            > MAX_DECOMPRESSED
                .saturating_sub(dest.len())
                .saturating_sub(6)
        {
            return Err("Localized SWF exceeds preview limits".into());
        }
        if updated == swf[at..at + length] {
            // Preserve the exact original tag header encoding, including a
            // long header on a tag that could also fit into a short one.
            dest.extend_from_slice(&swf[tag_begin..at + length]);
        } else {
            if updated.len() < 63 {
                dest.extend_from_slice(&((kind << 6) | updated.len() as u16).to_le_bytes());
            } else {
                dest.extend_from_slice(&((kind << 6) | 63).to_le_bytes());
                dest.extend_from_slice(&(updated.len() as u32).to_le_bytes());
            }
            dest.extend_from_slice(&updated);
        }
        at += length;
        count += 1;
        if kind == 0 {
            break;
        }
    }
    if at != swf.len() || count >= 80_000 {
        return Err("Unterminated or excessive main-menu SWF tags".into());
    }
    if dest.len() > MAX_DECOMPRESSED {
        return Err("Localized SWF exceeds preview limits".into());
    }
    let size = dest.len() as u32;
    dest[4..8].copy_from_slice(&size.to_le_bytes());
    Ok(dest)
}

/// Read only typed AVM1 ActionPush string constants, including the bodies of
/// ordinary DefineFunction actions. These original bytes are useful for
/// discovering runtime-assigned @ui_ keys without assuming screen labels.
/// Do not change original instruction lengths, jumps or authored bytecode.
fn avm1_localization_keys(actions: &[u8], depth: usize, keys: &mut BTreeSet<String>) {
    if depth > 4 || actions.len() > MAX_DECOMPRESSED {
        return;
    }
    let mut at = 0usize;
    let mut count = 0usize;
    while at < actions.len() && count < 32_000 && keys.len() < 256 {
        let code = actions[at];
        at += 1;
        if code == 0 {
            break;
        }
        let size = if code >= 0x80 {
            let Some(bytes) = actions.get(at..at + 2) else { break };
            at += 2;
            usize::from(u16::from_le_bytes([bytes[0], bytes[1]]))
        } else {
            0
        };
        let Some(end) = at.checked_add(size).filter(|&end| end <= actions.len()) else {
            break;
        };
        let payload = &actions[at..end];
        if code == 0x96 {
            let mut p = 0usize;
            while p < payload.len() {
                let ty = payload[p];
                p += 1;
                if ty == 0 {
                    let Some(stop) = zstring_end(payload, p) else { break };
                    if let Ok(value) = std::str::from_utf8(&payload[p..stop])
                        && value.to_ascii_lowercase().starts_with("@ui_")
                        && let Some(key) = label(value)
                    {
                        keys.insert(format!("@{key}"));
                    }
                    p = stop + 1;
                    continue;
                }
                let skip = match ty {
                    1 | 7 => 4,
                    2 | 3 => 0,
                    4 | 5 | 8 => 1,
                    6 => 8,
                    9 => 2,
                    _ => break,
                };
                let Some(next) = p.checked_add(skip).filter(|&next| next <= payload.len()) else {
                    break;
                };
                p = next;
            }
        } else if code == 0x9b && depth < 4 {
            // DefineFunction: functionName\0, parameter count, parameter
            // names, codeSize and the authored function bytecode.
            if let Some(name_end) = zstring_end(payload, 0) {
                let mut p = name_end + 1;
                if let Some(count_bytes) = payload.get(p..p + 2) {
                    let parameters = u16::from_le_bytes([count_bytes[0], count_bytes[1]]);
                    p += 2;
                    let mut valid = true;
                    for _ in 0..parameters {
                        if let Some(end) = zstring_end(payload, p) {
                            p = end + 1;
                        } else {
                            valid = false;
                            break;
                        }
                    }
                    if valid && let Some(size_bytes) = payload.get(p..p + 2) {
                        let code_size = usize::from(u16::from_le_bytes([
                            size_bytes[0], size_bytes[1],
                        ]));
                        p += 2;
                        if let Some(body) = payload.get(p..p.saturating_add(code_size)) {
                            avm1_localization_keys(body, depth + 1, keys);
                        }
                    }
                }
            }
        }
        at = end;
        count += 1;
    }
}

/// Extract keys from original edit-text initial values and typed AVM1
/// script strings; all translations remain source-qualified and exact-build.
pub fn authored_keys(swf: &[u8]) -> Result<Vec<String>, String> {
    let mut keys = BTreeSet::new();
    let _ = walk(swf, |kind, tag| {
        if kind == 37
            && let Some((start, end)) = edit_text_value(tag)
            && let Ok(raw) = std::str::from_utf8(&tag[start..end])
            && let Some(key) = label(raw)
        {
            keys.insert(format!("@{key}"));
        }
        if kind == 12 {
            avm1_localization_keys(tag, 0, &mut keys);
        } else if kind == 59 && tag.len() >= 2 {
            avm1_localization_keys(&tag[2..], 0, &mut keys);
        }
        Ok(tag.to_vec())
    })?;
    Ok(keys.into_iter().take(256).collect())
}

/// Translations must originate from exact-build text_ui_*.xml verification.
pub fn replace_authored_text(swf: &[u8], report: &Value) -> Result<(Vec<u8>, TextPatch), String> {
    let lookup = report.get("translations").and_then(Value::as_object);
    let mut diagnostics = TextPatch::default();
    let movie = walk(swf, |kind, tag| {
        if kind != 37 {
            return Ok(tag.to_vec());
        }
        diagnostics.original_edit_fields += 1;
        let Some((start, end)) = edit_text_value(tag) else {
            return Ok(tag.to_vec());
        };
        let Ok(raw) = std::str::from_utf8(&tag[start..end]) else {
            return Ok(tag.to_vec());
        };
        let Some(key) = label(raw) else {
            return Ok(tag.to_vec());
        };
        let replacement = lookup.and_then(|map| map.get(&key)).and_then(Value::as_str);
        let Some(replacement) = replacement else {
            diagnostics.unresolved += 1;
            return Ok(tag.to_vec());
        };
        if replacement.is_empty() || replacement.len() > 1024 || replacement.contains('\0') {
            diagnostics.unresolved += 1;
            return Ok(tag.to_vec());
        }
        diagnostics.patched += 1;
        let mut result = Vec::with_capacity(tag.len() + replacement.len());
        result.extend_from_slice(&tag[..start]);
        result.extend_from_slice(replacement.as_bytes());
        result.extend_from_slice(&tag[end..]);
        Ok(result)
    })?;
    Ok((movie, diagnostics))
}
#[cfg(test)]
mod tests {
    use super::*;
    // 1920x1080 @30 fps, one SWF DefineEditText and one unrelated DoAction.
    fn fixture(text: &str, flags: u16) -> Vec<u8> {
        let mut movie = b"FWS\x09".to_vec();
        movie.extend_from_slice(&[0; 4]);
        let mut bits = [0u8; 5 + 4 * 17];
        let mut put = |offset: usize, width: usize, value: u32| {
            for i in 0..width {
                bits[offset + i] = ((value >> (width - 1 - i)) & 1) as u8;
            }
        };
        put(0, 5, 17);
        for (i, value) in [0u32, 38400, 0, 21600].iter().enumerate() {
            put(5 + 17 * i, 17, *value);
        }
        for section in bits.chunks(8) {
            let byte = section.iter().fold(0u8, |byte, v| byte << 1 | *v);
            movie.push(byte << (8 - section.len()));
        }
        movie.extend_from_slice(&7680u16.to_le_bytes());
        movie.extend_from_slice(&1u16.to_le_bytes());
        let mut edit = vec![17, 0, 0b00001000, 0];
        edit.extend_from_slice(&flags.to_be_bytes());
        if flags & 0x0100 != 0 {
            edit.extend_from_slice(&[1, 0, 12, 0]);
        }
        edit.extend_from_slice(b"menuLabel\0");
        if flags & 0x8000 != 0 {
            edit.extend_from_slice(text.as_bytes());
            edit.push(0);
        }
        movie.extend_from_slice(&((37u16 << 6) | 63).to_le_bytes());
        movie.extend_from_slice(&(edit.len() as u32).to_le_bytes());
        movie.extend_from_slice(&edit);
        movie.extend_from_slice(&((12u16 << 6) | 4).to_le_bytes());
        movie.extend_from_slice(&[0x96, 1, 0, 0]); // unrelated DoAction bytes
        movie.extend_from_slice(&0u16.to_le_bytes());
        let length = movie.len() as u32;
        movie[4..8].copy_from_slice(&length.to_le_bytes());
        movie
    }
    #[test]
    fn verified_translation_only_patches_authored_text_not_script() {
        let before = fixture("@ui_STARTGAME", 0x8000 | 0x0100);
        assert_eq!(authored_keys(&before).unwrap(), vec!["@ui_startgame"]);
        let report = serde_json::json!({"translations": {"ui_startgame": "PLAY"}});
        let (after, info) = replace_authored_text(&before, &report).unwrap();
        assert_eq!(info.patched, 1);
        assert_eq!(info.unresolved, 0);
        assert!(after.windows(5).any(|part| part == b"PLAY\0"));
        assert!(after.windows(4).any(|part| part == [0x96, 1, 0, 0]));
        assert_eq!(&before[4..8], &(before.len() as u32).to_le_bytes());
        assert_eq!(&after[4..8], &(after.len() as u32).to_le_bytes());
        assert!(before.windows(13).any(|part| part == b"@ui_STARTGAME"));
    }
    #[test]
    fn unresolved_and_dynamic_fields_are_not_fabricated() {
        let original = fixture("@ui_STARTGAME", 0x8000);
        let (after, stats) = replace_authored_text(&original, &serde_json::json!({})).unwrap();
        assert_eq!(stats.unresolved, 1);
        assert_eq!(after, original);
        let dynamic = fixture("unused", 0);
        assert!(authored_keys(&dynamic).unwrap().is_empty());
        assert_eq!(
            replace_authored_text(&dynamic, &serde_json::json!({}))
                .unwrap()
                .0,
            dynamic
        );
    }
    #[test]
    fn discovers_runtime_actionscript_strings_without_mutating_bytecode() {
        let mut source = fixture("@ui_STARTGAME", 0x8000);
        let mut push = vec![0u8];
        push.extend_from_slice(b"@ui_SigningInPrompt\0");
        push.extend_from_slice(&[0u8]);
        push.extend_from_slice(b"not_a_localization_key\0");
        let mut actions = vec![0x96];
        actions.extend_from_slice(&(push.len() as u16).to_le_bytes());
        actions.extend_from_slice(&push);
        actions.push(0);
        let mut tag = ((12u16 << 6) | 63).to_le_bytes().to_vec();
        tag.extend_from_slice(&(actions.len() as u32).to_le_bytes());
        tag.extend_from_slice(&actions);
        let at = source.len() - 2;
        source.splice(at..at, tag);
        let size = source.len() as u32;
        source[4..8].copy_from_slice(&size.to_le_bytes());
        assert_eq!(
            authored_keys(&source).unwrap(),
            vec!["@ui_signinginprompt", "@ui_startgame"]
        );
        // Localization inventory never rewrites ActionScript instructions.
        let (preview, patch) =
            replace_authored_text(&source, &serde_json::json!({"translations":{
                "ui_startgame":"PLAY", "ui_signinginprompt":"Signing in"
            }})).unwrap();
        assert_eq!(patch.patched, 1);
        assert!(preview.windows(b"@ui_SigningInPrompt".len())
            .any(|part| part == b"@ui_SigningInPrompt"));
    }
    #[test]
    fn rejects_corrupt_swf_and_no_unsafe_guessing() {
        assert!(authored_keys(b"FWS").is_err());
        let mut broken = fixture("@ui_Play", 0x8000);
        broken[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(authored_keys(&broken).is_err());
        assert!(label("@ui_@ui_STARTGAME").is_none()); // unresolved composite key, never guessed
        assert!(label("@ui BAD").is_none());
    }
}
