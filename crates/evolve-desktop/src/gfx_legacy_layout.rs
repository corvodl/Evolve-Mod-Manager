//! Narrow, source-verified Stage 2 -> Legacy main-menu presentation preset.
//!
//! Stage 2 owns ActionScript, native callbacks, navigation and button creation.
//! Only three *existing* integer operands in its Style and ButtonMainMenu
//! DoInitAction classes may change. No tags are inserted, removed, reordered,
//! or rebuilt. This avoids
//! copying Legacy AVM bytecode or importing an incompatible whole movie.
use evolve_core::gfx::{MAX_SOURCE, inspect_gfx};
use std::collections::HashMap;

const STYLE: [(&str, i32, i32); 7] = [
    ("MAIN_MENU_BUTTON_FEATURED_WIDTH", 660, 660),
    ("MAIN_MENU_BUTTON_FEATURED_HEIGHT", 65, 70),
    ("MAIN_MENU_BUTTON_DEFAULT_WIDTH", 660, 660),
    ("MAIN_MENU_BUTTON_DEFAULT_HEIGHT", 40, 70),
    ("MAIN_MENU_BUTTON_WIDTH", 660, 660),
    ("MAIN_MENU_BUTTON_HEIGHT", 70, 70),
    ("MAIN_MENU_BUTTON_SPACING", 5, 5),
];

// Stage 2's ButtonMainMenu uses a 70px label inset. The authored Legacy
// Button class uses 85px; at 1280x720 the target screenshot starts the
// labels around x=58px (= 87 Stage coordinates). Keep other button values.
const MAIN_MENU_LABEL: [(&str, i32, i32); 5] = [
    ("DEFAULT_TEXT_HEIGHT", 40, 40),
    ("DEFAULT_TEXT_CENTER", 16, 16),
    ("DEFAULT_TEXT_POS_X", 70, 85),
    ("DEFAULT_TEXT_DESCRIPTION_WIDTH", 300, 300),
    ("HIGHLIGHT_FEATURED_POS_Y", -98, -98),
];

fn u16_at(data: &[u8], offset: usize) -> Result<usize, String> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or("Truncated GFX tag or AVM1 action length")?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]) as usize)
}
fn u32_at(data: &[u8], offset: usize) -> Result<usize, String> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or("Truncated long GFX tag length")?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
}

fn constant_pool(payload: &[u8]) -> Result<Vec<String>, String> {
    let count = u16_at(payload, 0)?;
    if count > 4096 {
        return Err("Unbounded Stage 2 ActionScript symbol pool".into());
    }
    let mut offset = 2usize;
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        let tail = payload
            .get(offset..)
            .ok_or("Truncated Stage 2 ActionScript constant pool")?;
        let end = tail
            .iter()
            .position(|value| *value == 0)
            .ok_or("Unterminated Stage 2 ActionScript symbol")?;
        names.push(
            std::str::from_utf8(&tail[..end])
                .map_err(|_| "Invalid Stage 2 ActionScript symbol encoding")?
                .to_owned(),
        );
        offset += end + 1;
    }
    if offset != payload.len() {
        return Err("Stage 2 ActionScript symbol pool has trailing data".into());
    }
    Ok(names)
}

/// The source's Style initializer uses Push register(1), constant pool index,
/// int32(value), followed immediately by SetMember (0x4f). No string search,
/// length-changing ActionScript reconstruction, or positional byte offsets.
fn style_setter(payload: &[u8], absolute: usize) -> Option<(usize, i32, usize)> {
    let (index, value) = match payload {
        [4, 1, 8, index, 7, a, b, c, d] => (*index as usize, i32::from_le_bytes([*a, *b, *c, *d])),
        [4, 1, 9, a, b, 7, c, d, e, f] => (
            u16::from_le_bytes([*a, *b]) as usize,
            i32::from_le_bytes([*c, *d, *e, *f]),
        ),
        _ => return None,
    };
    Some((index, value, absolute + payload.len() - 4))
}

fn style_tag(
    movie: &[u8],
    start: usize,
    end: usize,
    constants: &[(&'static str, i32, i32)],
) -> Result<Option<Vec<(usize, i32)>>, String> {
    let mut position = start + 2; // DoInitAction's sprite ID
    let mut pool = Vec::<String>::new();
    let mut fields = HashMap::<&'static str, (i32, usize)>::new();
    while position < end {
        let code = movie[position];
        position += 1;
        if code == 0 {
            break;
        }
        let length = if code >= 0x80 {
            let length = u16_at(movie, position)?;
            position += 2;
            length
        } else {
            0
        };
        let next = position
            .checked_add(length)
            .filter(|next| *next <= end)
            .ok_or("ActionScript instruction escapes its DoInitAction tag")?;
        let payload = &movie[position..next];
        if code == 0x88 {
            pool = constant_pool(payload)?;
        } else if code == 0x96
            && let Some((index, value, absolute)) = style_setter(payload, position)
            && let Some(name) = pool.get(index)
        {
            for &(field, _, _) in constants {
                if name == field {
                    if movie.get(next) != Some(&0x4f) {
                        return Err(
                            "Style constant lacks original AVM1 SetMember instruction".into()
                        );
                    }
                    if fields.insert(field, (value, absolute)).is_some() {
                        return Err(format!("Duplicate Stage 2 main-menu Style setter: {field}"));
                    }
                }
            }
        }
        position = next;
    }
    if fields.is_empty() {
        return Ok(None);
    }
    if fields.len() != constants.len() {
        return Err("Incomplete or unrecognized Stage 2 menu initializer".into());
    }
    let mut edits = Vec::new();
    for &(field, expected, replacement) in constants {
        let &(value, offset) = fields
            .get(field)
            .ok_or("Missing Stage 2 main-menu Style field")?;
        if value != expected {
            return Err(format!(
                "{field} is {value}, expected {expected}; refusing a different build or an already-applied preset"
            ));
        }
        if expected != replacement {
            edits.push((offset, replacement));
        }
    }
    Ok(Some(edits))
}

/// An opt-in visual preset for original Stage 2 main_menu.gfx only.
/// Every byte beyond the three identified int32 operands is retained.
pub fn legacy_main_menu_rows(movie: &[u8]) -> Result<Vec<u8>, String> {
    if movie.len() > MAX_SOURCE || !movie.starts_with(b"GFX") {
        return Err("Legacy row preset requires an uncompressed original Stage 2 GFX movie".into());
    }
    inspect_gfx(movie).map_err(|e| format!("Invalid original Stage 2 GFX: {e}"))?;
    let nbits = usize::from(movie[8] >> 3);
    let mut position = 8 + (5 + 4 * nbits).div_ceil(8) + 4; // RECT, frame rate/count
    let mut selected: Option<Vec<(usize, i32)>> = None;
    let mut menu_label: Option<Vec<(usize, i32)>> = None;
    while position < movie.len() {
        let header = u16_at(movie, position)?;
        position += 2;
        let code = header >> 6;
        let small_length = header & 63;
        let length = if small_length == 63 {
            let length = u32_at(movie, position)?;
            position += 4;
            length
        } else {
            small_length
        };
        let end = position
            .checked_add(length)
            .filter(|end| *end <= movie.len())
            .ok_or("Main-menu GFX tag exceeds file bounds")?;
        if code == 59 {
            if length < 2 {
                return Err("Truncated GFX DoInitAction sprite identifier".into());
            }
            let sprite = u16_at(movie, position)?;
            // Both class IDs are verified against the original Stage 2
            // Main Menu movie (Style=285; ButtonMainMenu=327). Reject other
            // builds instead of accidentally changing an unrelated script.
            if sprite == 285
                && let Some(edits) = style_tag(movie, position, end, &STYLE)?
                && selected.replace(edits).is_some()
            {
                return Err("Ambiguous Stage 2 main-menu Style initializer".into());
            }
            if sprite == 327
                && let Some(edits) = style_tag(movie, position, end, &MAIN_MENU_LABEL)?
                && menu_label.replace(edits).is_some()
            {
                return Err("Ambiguous Stage 2 ButtonMainMenu label initializer".into());
            }
        }
        position = end;
        if code == 0 {
            break;
        }
    }
    let mut edits = selected.ok_or(
        "This movie lacks the expected Stage 2 authored button styles. Do not apply this preset to a Legacy movie.",
    )?;
    edits.extend(menu_label.ok_or("Missing original Stage 2 ButtonMainMenu label initializer")?);
    if edits.len() != 3 {
        return Err(
            "Stage 2 main-menu preset did not identify exactly three style operands".into(),
        );
    }
    let mut updated = movie.to_vec();
    for (offset, value) in edits {
        updated[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    inspect_gfx(&updated).map_err(|e| format!("Rewritten GFX failed validation: {e}"))?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn write_tag(data: &mut Vec<u8>, code: u16, payload: &[u8]) {
        data.extend(((code << 6) | 63).to_le_bytes());
        data.extend((payload.len() as u32).to_le_bytes());
        data.extend(payload);
    }
    fn fixture(alter_default: Option<i32>, duplicate: bool) -> Vec<u8> {
        // GFX version 15, 1920x1080 stage, 30 fps, one frame.
        let numbers = [0i32, 38400, 0, 21600];
        let mut bits = [0u8; 5 + 4 * 17];
        for (index, bit) in (0..5).rev().enumerate() {
            bits[index] = (17u32 >> bit & 1) as u8;
        }
        for (index, number) in numbers.into_iter().enumerate() {
            for bit in 0..17 {
                bits[5 + index * 17 + bit] = ((number as u32 >> (16 - bit)) & 1) as u8;
            }
        }
        let mut frame = vec![0u8; bits.len().div_ceil(8)];
        for (index, bit) in bits.into_iter().enumerate() {
            frame[index / 8] |= bit << (7 - index % 8);
        }
        frame.extend(7680u16.to_le_bytes());
        frame.extend(1u16.to_le_bytes());
        // This unrelated DoAction must survive the preset byte-for-byte.
        write_tag(&mut frame, 12, &[0x96, 0, 0, 0]);
        let mut script = 285u16.to_le_bytes().to_vec();
        let mut pool = (STYLE.len() as u16).to_le_bytes().to_vec();
        for (field, _, _) in STYLE {
            pool.extend(field.as_bytes());
            pool.push(0);
        }
        script.push(0x88);
        script.extend((pool.len() as u16).to_le_bytes());
        script.extend(pool);
        for (index, (_, old, _)) in STYLE.into_iter().enumerate() {
            let value = if index == 3 {
                alter_default.unwrap_or(old)
            } else {
                old
            };
            script.extend([0x96, 9, 0, 4, 1, 8, index as u8, 7]);
            script.extend(value.to_le_bytes());
            script.push(0x4f);
        }
        script.push(0); // AVM1 ActionEnd
        write_tag(&mut frame, 59, &script);
        if duplicate {
            write_tag(&mut frame, 59, &script);
        }
        let mut label_script = 327u16.to_le_bytes().to_vec();
        let mut label_pool = (MAIN_MENU_LABEL.len() as u16).to_le_bytes().to_vec();
        for (field, _, _) in MAIN_MENU_LABEL {
            label_pool.extend(field.as_bytes());
            label_pool.push(0);
        }
        label_script.push(0x88);
        label_script.extend((label_pool.len() as u16).to_le_bytes());
        label_script.extend(label_pool);
        for (index, (_, old, _)) in MAIN_MENU_LABEL.into_iter().enumerate() {
            label_script.extend([0x96, 9, 0, 4, 1, 8, index as u8, 7]);
            label_script.extend(old.to_le_bytes());
            label_script.push(0x4f);
        }
        label_script.push(0);
        write_tag(&mut frame, 59, &label_script);
        frame.extend([0, 0]); // End GFX tag
        let mut gfx = b"GFX\x0f".to_vec();
        gfx.extend(((frame.len() + 8) as u32).to_le_bytes());
        gfx.extend(frame);
        gfx
    }
    #[test]
    fn edits_only_two_style_operands_without_touching_any_other_gfx_tags() {
        let original = fixture(None, false);
        let updated = legacy_main_menu_rows(&original).unwrap();
        assert_eq!(updated.len(), original.len());
        let different: Vec<_> = original
            .iter()
            .zip(&updated)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(different.len(), 3); // 40/65->70 rows, 70->85 text X, int32 little endian
        assert!(updated.windows(4).any(|data| data == 70i32.to_le_bytes()));
        assert!(legacy_main_menu_rows(&updated).is_err()); // do not stage twice
    }
    #[test]
    fn refuses_other_builds_and_ambiguous_styles() {
        assert!(legacy_main_menu_rows(&fixture(Some(39), false)).is_err());
        assert!(legacy_main_menu_rows(&fixture(None, true)).is_err());
        let legacy = b"GFX\x0a".to_vec();
        assert!(legacy_main_menu_rows(&legacy).is_err());
    }
}
