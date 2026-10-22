//! Bounded numeric/boolean XML editing without invented gameplay defaults.
use quick_xml::{Reader, Writer, events::Event};

pub const LIMIT: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct ValueField {
    pub element: usize,
    pub attribute: String,
    pub label: String,
    pub original: String,
    pub value: String,
}
fn numeric_shape(value: &str) -> Option<Vec<usize>> {
    value
        .split('|')
        .map(|level| {
            let components: Vec<_> = level.split(',').collect();
            components
                .iter()
                .all(|s| s.trim().parse::<f64>().is_ok_and(f64::is_finite))
                .then_some(components.len())
        })
        .collect()
}
fn scalar(value: &str) -> bool {
    matches!(value, "true" | "false") || numeric_shape(value).is_some()
}
pub fn fields(bytes: &[u8]) -> Result<Vec<ValueField>, String> {
    // Reuse the strict XML root/depth/entity checks used by the signed builder.
    crate::cryxml::encode_xml(bytes, LIMIT).map_err(|e| e.to_string())?;
    let mut reader = Reader::from_reader(bytes);
    let mut out = Vec::new();
    let mut element = 0;
    let mut parents = Vec::new();
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) | Event::Empty(e) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let attrs: Vec<_> = e
                    .attributes()
                    .map(|a| {
                        let a = a.map_err(|e| e.to_string())?;
                        Ok((
                            String::from_utf8_lossy(a.key.as_ref()).into_owned(),
                            a.decode_and_unescape_value(reader.decoder())
                                .map_err(|e| e.to_string())?
                                .into_owned(),
                        ))
                    })
                    .collect::<Result<_, String>>()?;
                let name = attrs
                    .iter()
                    .find(|(k, _)| k == "name" || k == "Name")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or(&tag);
                for (key, value) in &attrs {
                    if scalar(value) {
                        if out.len() == 10000 {
                            return Err("Too many editable fields".into());
                        }
                        out.push(ValueField {
                            element,
                            attribute: key.clone(),
                            label: format!(
                                "{} / {name} · {key} [element {}]",
                                parents.join(" / "),
                                element + 1
                            ),
                            original: value.clone(),
                            value: value.clone(),
                        });
                    }
                }
                // Reader slice positions distinguish the empty tag without changing XML.
                let end = reader.buffer_position() as usize;
                if !bytes[..end].ends_with(b"/>") {
                    parents.push(name.to_string());
                }
                element += 1;
            }
            Event::End(_) => {
                parents.pop();
            }
            Event::Eof => break,
            _ => (),
        }
    }
    Ok(out)
}
pub fn apply(bytes: &[u8], edits: &[ValueField]) -> Result<Vec<u8>, String> {
    let expected = fields(bytes)?;
    if expected.len() != edits.len()
        || expected.iter().zip(edits).any(|(a, b)| {
            a.element != b.element || a.attribute != b.attribute || a.original != b.original
        })
    {
        return Err("Field selection no longer matches the source document".into());
    }
    for field in edits {
        let valid = if matches!(field.original.as_str(), "true" | "false") {
            matches!(field.value.as_str(), "true" | "false")
        } else {
            numeric_shape(&field.value).is_some()
                && numeric_shape(&field.value) == numeric_shape(&field.original)
        };
        if !valid {
            return Err(format!(
                "{} needs a finite number or its original boolean type",
                field.label
            ));
        }
    }
    let mut reader = Reader::from_reader(bytes);
    let mut writer = Writer::new(Vec::new());
    let mut changed: std::collections::HashMap<usize, Vec<&ValueField>> =
        std::collections::HashMap::new();
    for f in edits.iter().filter(|f| f.value != f.original) {
        changed.entry(f.element).or_default().push(f);
    }
    let mut element = 0;
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        let event = match event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = bytes[..reader.buffer_position() as usize].ends_with(b"/>");
                let changes = changed.get(&element).map(Vec::as_slice).unwrap_or(&[]);
                element += 1;
                let e = if changes.is_empty() {
                    e.into_owned()
                } else {
                    let mut updated = e.clone().into_owned();
                    updated.clear_attributes();
                    for attr in e.attributes() {
                        let attr = attr.map_err(|e| e.to_string())?;
                        if let Some(f) = changes
                            .iter()
                            .find(|f| f.attribute.as_bytes() == attr.key.as_ref())
                        {
                            updated.push_attribute((f.attribute.as_str(), f.value.as_str()));
                        } else {
                            updated.push_attribute(attr);
                        }
                    }
                    updated
                };
                if empty {
                    Event::Empty(e)
                } else {
                    Event::Start(e)
                }
            }
            Event::Eof => break,
            e => e.into_owned(),
        };
        writer.write_event(event).map_err(|e| e.to_string())?;
    }
    let result = writer.into_inner();
    crate::cryxml::encode_xml(&result, LIMIT).map_err(|e| e.to_string())?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_params_edit_only_selected_value_and_preserve_strings() {
        let xml = br#"<ability name="A"><params><param name="cooldown" value="11"/><param name="cooldown" value="22"/><param name="icon" value="a&amp;b"/></params></ability>"#;
        let mut f = fields(xml).unwrap();
        assert_eq!(f.len(), 2);
        assert!(f[0].label.contains("cooldown"));
        f[1].value = "3.5".into();
        let edited = apply(xml, &f).unwrap();
        let reloaded = fields(&edited).unwrap();
        assert_eq!(reloaded[0].value, "11");
        assert_eq!(reloaded[1].value, "3.5");
        assert!(String::from_utf8(edited).unwrap().contains("a&amp;b"));
        f[0].value = "NaN".into();
        assert!(apply(xml, &f).is_err());
    }
    #[test]
    fn staged_vectors_preserve_component_counts() {
        let xml = br#"<Actor><param name="size" value="1, 2, 3|4, 5, 6"/></Actor>"#;
        let mut f = fields(xml).unwrap();
        f[0].value = "2, 3, 4|5, 6, 7".into();
        assert!(apply(xml, &f).is_ok());
        for invalid in ["1|2", "1,2,3", "1,2,3|4,NaN,6"] {
            f[0].value = invalid.into();
            assert!(apply(xml, &f).is_err());
        }
    }
    #[test]
    #[ignore = "requires a local user-owned gameplay document"]
    fn real_gameplay_document_round_trip() {
        let path = std::env::var("EVOLVE_GAMEPLAY_DOCUMENT").expect("set EVOLVE_GAMEPLAY_DOCUMENT");
        let bytes = std::fs::read(path).unwrap();
        let f = fields(&bytes).unwrap();
        assert!(!f.is_empty());
        let result = apply(&bytes, &f).unwrap();
        assert_eq!(result, bytes, "unchanged XML must stay byte-identical");
        eprintln!("{} editable numeric/boolean fields validated", f.len());
    }
    #[test]
    fn rejects_bad_xml_and_stale_fields() {
        assert!(fields(b"<a><b></a>").is_err());
        let a = b"<a enabled=\"true\"/>";
        let mut f = fields(a).unwrap();
        f[0].value = "1".into();
        assert!(apply(a, &f).is_err());
        f[0].value = "false".into();
        assert!(apply(b"<a enabled=\"false\"/>", &f).is_err());
    }
}
