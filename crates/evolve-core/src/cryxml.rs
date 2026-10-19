//! Bounded CryEngine 3 binary XML reader. Original bytes remain untouched.
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use std::borrow::Cow;
use std::collections::HashSet;

#[derive(Debug, thiserror::Error)]
#[error("Invalid or excessive CryXmlB: {0}")]
pub struct CryXmlError(pub &'static str);

fn u32_at(data: &[u8], offset: usize) -> Result<usize, CryXmlError> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or(CryXmlError("truncated integer"))?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
}

fn xml_text(value: &str) -> bool {
    value.chars().all(|c| matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))
}

fn xml_name(value: &str) -> bool {
    let start = |c: char| c == '_' || c == ':' || c.is_alphabetic();
    let mut chars = value.chars();
    chars.next().is_some_and(start)
        && chars.all(|c| start(c) || c.is_numeric() || c == '-' || c == '.')
}

/// Accept text XML unchanged or decode a binary document within the caller's
/// byte budget. Bound both input and expanded output (strings may be shared).
pub fn xml_bytes(data: &[u8], limit: usize) -> Result<Cow<'_, [u8]>, CryXmlError> {
    if data.len() > limit {
        return Err(CryXmlError("input exceeds byte limit"));
    }
    if !data.starts_with(b"CryXmlB\0") {
        return Ok(Cow::Borrowed(data));
    }
    if data.len() < 44 || u32_at(data, 8)? != data.len() {
        return Err(CryXmlError("file size mismatch"));
    }
    let mut tables = Vec::new();
    for (field, width) in [(12, 28), (20, 8), (28, 4), (36, 1)] {
        let offset = u32_at(data, field)?;
        let count = u32_at(data, field + 4)?;
        let size = count
            .checked_mul(width)
            .ok_or(CryXmlError("table overflow"))?;
        let end = offset
            .checked_add(size)
            .ok_or(CryXmlError("table overflow"))?;
        if offset < 44 || end > data.len() {
            return Err(CryXmlError("table outside file"));
        }
        tables.push((offset, count, end));
    }
    for (i, &(a, _, b)) in tables.iter().enumerate() {
        for &(c, _, d) in &tables[i + 1..] {
            if a < b && c < d && a < d && c < b {
                return Err(CryXmlError("overlapping tables"));
            }
        }
    }
    let (nodes_at, node_count, _) = tables[0];
    let (attrs_at, attr_count, _) = tables[1];
    let (children_at, child_count, _) = tables[2];
    let (strings_at, strings_len, _) = tables[3];
    if node_count == 0 || node_count > 200_000 {
        return Err(CryXmlError("node count limit"));
    }
    let string = |index: usize| -> Result<&str, CryXmlError> {
        if index >= strings_len {
            return Err(CryXmlError("string offset outside table"));
        }
        let tail = &data[strings_at + index..strings_at + strings_len];
        let end = tail
            .iter()
            .position(|&b| b == 0)
            .ok_or(CryXmlError("unterminated string"))?;
        let value =
            std::str::from_utf8(&tail[..end]).map_err(|_| CryXmlError("non-UTF8 string"))?;
        if !xml_text(value) {
            return Err(CryXmlError("invalid XML character"));
        }
        Ok(value)
    };
    let mut roots = Vec::new();
    for i in 0..node_count {
        if u32_at(data, nodes_at + i * 28 + 12)? == u32::MAX as usize {
            roots.push(i);
        }
    }
    if roots.len() != 1 {
        return Err(CryXmlError("expected one root"));
    }
    let mut writer = Writer::new(Vec::new());
    let mut visited = vec![false; node_count];
    let mut stack = vec![(roots[0], false, 0)];
    while let Some((index, closing, depth)) = stack.pop() {
        if depth > 128 {
            return Err(CryXmlError("tree depth limit"));
        }
        let base = nodes_at + index * 28;
        let name = string(u32_at(data, base)?)?;
        if !xml_name(name) {
            return Err(CryXmlError("invalid XML name"));
        }
        if closing {
            writer
                .write_event(Event::End(BytesEnd::new(name)))
                .map_err(|_| CryXmlError("XML write failed"))?;
        } else {
            if visited[index] {
                return Err(CryXmlError("cycle or multiple parents"));
            }
            visited[index] = true;
            let counts = u32_at(data, base + 8)?;
            let na = counts & 0xffff;
            let nc = counts >> 16;
            let fa = u32_at(data, base + 16)?;
            let fc = u32_at(data, base + 20)?;
            if fa.checked_add(na).is_none_or(|end| end > attr_count)
                || fc.checked_add(nc).is_none_or(|end| end > child_count)
            {
                return Err(CryXmlError("invalid node range"));
            }
            let mut tag = BytesStart::new(name);
            let mut keys = HashSet::new();
            let mut estimated = name.len().saturating_add(5);
            for j in fa..fa + na {
                let key = string(u32_at(data, attrs_at + j * 8)?)?;
                let value = string(u32_at(data, attrs_at + j * 8 + 4)?)?;
                if !xml_name(key) || !keys.insert(key) {
                    return Err(CryXmlError("invalid or duplicate attribute"));
                }
                estimated = estimated
                    .saturating_add(key.len())
                    .saturating_add(value.len().saturating_mul(6))
                    .saturating_add(4);
                if writer.get_ref().len().saturating_add(estimated) > limit {
                    return Err(CryXmlError("expanded XML limit"));
                }
                tag.push_attribute((key, value));
            }
            let content = string(u32_at(data, base + 4)?)?;
            if writer
                .get_ref()
                .len()
                .saturating_add(estimated)
                .saturating_add(content.len().saturating_mul(6))
                > limit
            {
                return Err(CryXmlError("expanded XML limit"));
            }
            writer
                .write_event(Event::Start(tag))
                .map_err(|_| CryXmlError("XML write failed"))?;
            writer
                .write_event(Event::Text(BytesText::new(content)))
                .map_err(|_| CryXmlError("XML write failed"))?;
            stack.push((index, true, depth));
            for j in (fc..fc + nc).rev() {
                let child = u32_at(data, children_at + j * 4)?;
                if child >= node_count || u32_at(data, nodes_at + child * 28 + 12)? != index {
                    return Err(CryXmlError("parent/child mismatch"));
                }
                stack.push((child, false, depth + 1));
            }
            if stack.len() > node_count.saturating_mul(2) {
                return Err(CryXmlError("tree traversal limit"));
            }
        }
        if writer.get_ref().len() > limit {
            return Err(CryXmlError("expanded XML limit"));
        }
    }
    if visited.contains(&false) {
        return Err(CryXmlError("unreachable nodes"));
    }
    Ok(Cow::Owned(writer.into_inner()))
}
