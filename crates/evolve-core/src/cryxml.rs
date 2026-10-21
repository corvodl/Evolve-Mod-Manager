//! Bounded CryEngine 3 binary XML reader. Original bytes remain untouched.
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

#[derive(Debug, PartialEq)]
struct XmlNode {
    name: String,
    content: String,
    attributes: Vec<(String, String)>,
    children: Vec<usize>,
    parent: Option<usize>,
}

fn document(bytes: &[u8], limit: usize) -> Result<Vec<XmlNode>, CryXmlError> {
    if bytes.is_empty() || bytes.len() > limit {
        return Err(CryXmlError("XML input limit"));
    }
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().expand_empty_elements = true;
    let mut nodes: Vec<XmlNode> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|_| CryXmlError("malformed editable XML"))?;
        match event {
            Event::Start(e) => {
                if nodes.len() >= 200_000
                    || stack.len() > 128
                    || (stack.is_empty() && !nodes.is_empty())
                {
                    return Err(CryXmlError("XML root/node/depth limit"));
                }
                let name = std::str::from_utf8(e.name().as_ref())
                    .map_err(|_| CryXmlError("invalid tag UTF-8"))?
                    .to_owned();
                if !xml_name(&name) {
                    return Err(CryXmlError("invalid XML name"));
                }
                let mut attributes = Vec::new();
                let mut seen = HashSet::new();
                for a in e.attributes() {
                    let a = a.map_err(|_| CryXmlError("invalid or duplicate attribute"))?;
                    let key = std::str::from_utf8(a.key.as_ref())
                        .map_err(|_| CryXmlError("invalid attribute UTF-8"))?
                        .to_owned();
                    // XML normalizes literal attribute whitespace before
                    // entity expansion; numeric tab/newline references must
                    // retain their authored characters.
                    let raw = std::str::from_utf8(&a.value)
                        .map_err(|_| CryXmlError("invalid attribute UTF-8"))?;
                    let normalized: String = raw
                        .replace("\r\n", "\n")
                        .chars()
                        .map(|c| {
                            if matches!(c, '\t' | '\r' | '\n') {
                                ' '
                            } else {
                                c
                            }
                        })
                        .collect();
                    let value = quick_xml::escape::unescape(&normalized)
                        .map_err(|_| CryXmlError("invalid attribute value"))?
                        .into_owned();
                    if !xml_name(&key)
                        || !seen.insert(key.clone())
                        || !xml_text(&value)
                        || attributes.len() >= u16::MAX as usize
                    {
                        return Err(CryXmlError("invalid or excessive attributes"));
                    }
                    attributes.push((key, value));
                }
                let index = nodes.len();
                if let Some(&parent) = stack.last() {
                    if nodes[parent].children.len() >= u16::MAX as usize {
                        return Err(CryXmlError("child count limit"));
                    }
                    nodes[parent].children.push(index);
                }
                nodes.push(XmlNode {
                    name,
                    attributes,
                    content: String::new(),
                    children: Vec::new(),
                    parent: stack.last().copied(),
                });
                stack.push(index);
            }
            Event::End(e) => {
                let index = stack.pop().ok_or(CryXmlError("extra XML closing tag"))?;
                if nodes[index].name.as_bytes() != e.name().as_ref() {
                    return Err(CryXmlError("mismatched XML tag"));
                }
                if nodes[index].content.trim().is_empty() {
                    nodes[index].content.clear();
                }
            }
            Event::Text(e) => {
                let text = e
                    .xml_content()
                    .map_err(|_| CryXmlError("invalid text encoding"))?;
                append_content(&mut nodes, &stack, &text)?;
            }
            Event::CData(e) => {
                let text = e
                    .decode()
                    .map_err(|_| CryXmlError("invalid CDATA encoding"))?;
                append_content(&mut nodes, &stack, &text)?;
            }
            Event::GeneralRef(e) => {
                let reference =
                    std::str::from_utf8(e.as_ref()).map_err(|_| CryXmlError("invalid entity"))?;
                let escaped = format!("&{reference};");
                let text = quick_xml::escape::unescape(&escaped)
                    .map_err(|_| CryXmlError("unsupported entity"))?;
                append_content(&mut nodes, &stack, &text)?;
            }
            Event::Decl(e) => {
                if !nodes.is_empty()
                    || e.encoding()
                        .transpose()
                        .map_err(|_| CryXmlError("invalid XML declaration"))?
                        .is_some_and(|v| !v.eq_ignore_ascii_case(b"utf-8"))
                {
                    return Err(CryXmlError("only UTF-8 XML declarations are supported"));
                }
            }
            Event::Eof => break,
            // Binary XML cannot represent DTDs, instructions or comments.
            _ => return Err(CryXmlError("unsupported XML construct")),
        }
    }
    if nodes.is_empty() || !stack.is_empty() {
        return Err(CryXmlError("incomplete XML document"));
    }
    Ok(nodes)
}

fn append_content(nodes: &mut [XmlNode], stack: &[usize], text: &str) -> Result<(), CryXmlError> {
    if !xml_text(text) {
        return Err(CryXmlError("invalid XML character"));
    }
    let Some(&index) = stack.last() else {
        return if text.trim().is_empty() {
            Ok(())
        } else {
            Err(CryXmlError("text outside root"))
        };
    };
    if !nodes[index].children.is_empty() {
        if !text.trim().is_empty() {
            return Err(CryXmlError(
                "mixed-content tails cannot be represented in CryXmlB",
            ));
        }
    } else {
        nodes[index].content.push_str(text);
    }
    Ok(())
}

/// Serialize editable UTF-8 XML into CryEngine 3 node/attribute/child/string
/// tables. Qualified names, xmlns declarations and attribute order survive.
/// Reject XML features which cannot round-trip through the binary format.
pub fn encode_xml(bytes: &[u8], limit: usize) -> Result<Vec<u8>, CryXmlError> {
    let nodes = document(bytes, limit)?;
    let mut strings = vec![0u8];
    let mut interned = HashMap::from([(String::new(), 0u32)]);
    let mut intern = |value: &str| -> Result<u32, CryXmlError> {
        if let Some(&offset) = interned.get(value) {
            return Ok(offset);
        }
        if strings.len().saturating_add(value.len()).saturating_add(1) > limit {
            return Err(CryXmlError("string table limit"));
        }
        let offset =
            u32::try_from(strings.len()).map_err(|_| CryXmlError("string table overflow"))?;
        strings.extend_from_slice(value.as_bytes());
        strings.push(0);
        interned.insert(value.to_owned(), offset);
        Ok(offset)
    };
    let mut records = Vec::new();
    let mut attributes = Vec::new();
    let mut children = Vec::new();
    for node in &nodes {
        records.extend_from_slice(&intern(&node.name)?.to_le_bytes());
        records.extend_from_slice(&intern(&node.content)?.to_le_bytes());
        records.extend_from_slice(&(node.attributes.len() as u16).to_le_bytes());
        records.extend_from_slice(&(node.children.len() as u16).to_le_bytes());
        for v in [
            node.parent.map_or(u32::MAX, |i| i as u32),
            (attributes.len() / 8) as u32,
            (children.len() / 4) as u32,
            0,
        ] {
            records.extend_from_slice(&v.to_le_bytes());
        }
        for (key, value) in &node.attributes {
            attributes.extend_from_slice(&intern(key)?.to_le_bytes());
            attributes.extend_from_slice(&intern(value)?.to_le_bytes());
        }
        for &child in &node.children {
            children.extend_from_slice(&(child as u32).to_le_bytes());
        }
        if records
            .len()
            .saturating_add(attributes.len())
            .saturating_add(children.len())
            > limit
        {
            return Err(CryXmlError("binary table limit"));
        }
    }
    let attrs_at = 44 + records.len();
    let children_at = attrs_at + attributes.len();
    let strings_at = children_at + children.len();
    let total = strings_at
        .checked_add(strings.len())
        .ok_or(CryXmlError("binary size overflow"))?;
    if total > limit {
        return Err(CryXmlError("binary size limit"));
    }
    let mut output = b"CryXmlB\0".to_vec();
    for v in [
        total,
        44,
        nodes.len(),
        attrs_at,
        attributes.len() / 8,
        children_at,
        children.len() / 4,
        strings_at,
        strings.len(),
    ] {
        output.extend_from_slice(
            &u32::try_from(v)
                .map_err(|_| CryXmlError("binary header overflow"))?
                .to_le_bytes(),
        );
    }
    output.extend(records);
    output.extend(attributes);
    output.extend(children);
    output.extend(strings);
    let decoded = xml_bytes(&output, limit)?;
    if document(&decoded, limit)? != nodes {
        return Err(CryXmlError("semantic round-trip mismatch"));
    }
    Ok(output)
}

/// Verify an edited XML document retains its original binary root identity.
pub fn encode_edited_xml(
    original: &[u8],
    edited: &[u8],
    limit: usize,
) -> Result<Vec<u8>, CryXmlError> {
    if !original.starts_with(b"CryXmlB\0") {
        return Err(CryXmlError("original is not CryXmlB"));
    }
    let decoded = xml_bytes(original, limit)?;
    let old = document(&decoded, limit)?;
    let new = document(edited, limit)?;
    if old[0].name != new[0].name {
        return Err(CryXmlError("edited root differs from original"));
    }
    encode_xml(edited, limit)
}

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
                let escaped = quick_xml::escape::escape(value)
                    .replace('\t', "&#9;")
                    .replace('\n', "&#10;")
                    .replace('\r', "&#13;");
                tag.push_attribute(quick_xml::events::attributes::Attribute {
                    key: quick_xml::name::QName(key.as_bytes()),
                    value: Cow::Owned(escaped.into_bytes()),
                });
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
                .write_event(Event::Text(BytesText::from_escaped(
                    quick_xml::escape::escape(content).replace('\r', "&#13;"),
                )))
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
