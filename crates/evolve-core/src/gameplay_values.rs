//! Bounded numeric/boolean XML and CryXmlB editing without invented gameplay defaults.
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
#[derive(Clone, Debug)]
pub struct SourceField {
    pub label: String,
    pub value: String,
}

/// Inspect every authored XML attribute and non-empty text node without
/// implying that every field is safe to edit. Editing has stricter shape rules.
pub fn source_fields(bytes: &[u8]) -> Result<Vec<SourceField>, String> {
    let xml = crate::cryxml::xml_bytes(bytes, LIMIT).map_err(|error| error.to_string())?;
    crate::cryxml::encode_xml(&xml, LIMIT).map_err(|error| error.to_string())?;
    let mut reader = Reader::from_reader(xml.as_ref());
    let mut parents = Vec::<String>::new();
    let mut fields = Vec::new();
    let mut text_value = String::new();
    let flush_text = |fields: &mut Vec<SourceField>,
                      parents: &[String],
                      text_value: &mut String|
     -> Result<(), String> {
        if !text_value.trim().is_empty() {
            if fields.len() >= 20_000 {
                return Err("Too many source fields".into());
            }
            fields.push(SourceField {
                label: format!("{} · text", parents.join(" / ")),
                value: text_value.clone(),
            });
        }
        text_value.clear();
        Ok(())
    };
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(tag) | Event::Empty(tag) => {
                flush_text(&mut fields, &parents, &mut text_value)?;
                let empty = xml[..reader.buffer_position() as usize].ends_with(b"/>");
                let element = std::str::from_utf8(tag.name().as_ref())
                    .map_err(|error| error.to_string())?
                    .to_owned();
                let mut attributes = Vec::new();
                for attribute in tag.attributes() {
                    let attribute = attribute.map_err(|error| error.to_string())?;
                    attributes.push((
                        std::str::from_utf8(attribute.key.as_ref())
                            .map_err(|error| error.to_string())?
                            .to_owned(),
                        attribute
                            .decode_and_unescape_value(reader.decoder())
                            .map_err(|error| error.to_string())?
                            .into_owned(),
                    ));
                }
                let label = attributes
                    .iter()
                    .find(|(key, _)| key == "name" || key == "Name")
                    .map_or(element.as_str(), |(_, value)| value.as_str())
                    .to_owned();
                parents.push(label);
                for (key, value) in attributes {
                    if fields.len() >= 20_000 {
                        return Err("Too many source fields".into());
                    }
                    fields.push(SourceField {
                        label: format!("{} · {key}", parents.join(" / ")),
                        value,
                    });
                }
                if empty {
                    parents.pop();
                }
            }
            Event::End(_) => {
                flush_text(&mut fields, &parents, &mut text_value)?;
                parents.pop();
            }
            Event::Text(text) => {
                let value = text.xml_content().map_err(|error| error.to_string())?;
                text_value.push_str(&value);
            }
            Event::CData(text) => {
                let value = text.decode().map_err(|error| error.to_string())?;
                text_value.push_str(&value);
            }
            Event::GeneralRef(reference) => {
                let reference =
                    std::str::from_utf8(reference.as_ref()).map_err(|error| error.to_string())?;
                let escaped = format!("&{reference};");
                text_value.push_str(
                    &quick_xml::escape::unescape(&escaped).map_err(|error| error.to_string())?,
                );
            }
            Event::Eof => {
                flush_text(&mut fields, &parents, &mut text_value)?;
                break;
            }
            _ => (),
        }
    }
    Ok(fields)
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
fn boolean(value: &str) -> bool {
    value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false")
}
fn scalar(value: &str) -> bool {
    boolean(value) || numeric_shape(value).is_some()
}
/// Enumerate numeric/boolean attributes from UTF-8 XML or CryXmlB. Binary
/// input is decoded in memory only; callers keep the original bytes as their
/// save baseline.
pub fn fields(bytes: &[u8]) -> Result<Vec<ValueField>, String> {
    // Accept authored UTF-8 XML or CryXmlB. Binary documents are decoded in
    // memory only; callers keep the original bytes as their save baseline.
    let xml = crate::cryxml::xml_bytes(bytes, LIMIT).map_err(|e| e.to_string())?;
    // Reuse the strict XML root/depth/entity checks used by the signed builder.
    crate::cryxml::encode_xml(&xml, LIMIT).map_err(|e| e.to_string())?;
    parse_fields(&xml)
}
fn parse_fields(bytes: &[u8]) -> Result<Vec<ValueField>, String> {
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
/// Apply validated edits. Text XML stays text; CryXmlB is rebuilt as CryXmlB
/// with its original root identity. Unchanged fields return the input bytes
/// exactly, so an untouched save cannot alter formatting or binary tables.
pub fn apply(bytes: &[u8], edits: &[ValueField]) -> Result<Vec<u8>, String> {
    let xml = crate::cryxml::xml_bytes(bytes, LIMIT).map_err(|e| e.to_string())?;
    crate::cryxml::encode_xml(&xml, LIMIT).map_err(|e| e.to_string())?;
    let expected = parse_fields(&xml)?;
    if expected.len() != edits.len()
        || expected.iter().zip(edits).any(|(a, b)| {
            a.element != b.element || a.attribute != b.attribute || a.original != b.original
        })
    {
        return Err("Field selection no longer matches the source document".into());
    }
    for field in edits {
        let valid = if boolean(&field.original) {
            boolean(&field.value)
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
    // An all-original edit set must not rewrite formatting or binary tables.
    if edits.iter().all(|f| f.value == f.original) {
        return Ok(bytes.to_vec());
    }
    let mut reader = Reader::from_reader(&xml[..]);
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
                let empty = xml[..reader.buffer_position() as usize].ends_with(b"/>");
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
    if bytes.starts_with(b"CryXmlB\0") {
        crate::cryxml::encode_edited_xml(bytes, &result, LIMIT).map_err(|e| e.to_string())
    } else {
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_inspection_preserves_text_references_and_numbers_without_enabling_them_for_edit() {
        let xml = br#"<Actor name="Maggie" base="BaseMerc"><param name="sItem" value="MobileArenaTriggerWeapon"/><param name="fMaxHealth" value="200"/><note>Inherited &amp; shared</note></Actor>"#;
        let source = source_fields(xml).unwrap();
        assert!(source.iter().any(
            |field| field.label.contains("sItem") && field.value == "MobileArenaTriggerWeapon"
        ));
        assert!(
            source
                .iter()
                .any(|field| field.label.contains("base") && field.value == "BaseMerc")
        );
        assert!(
            source
                .iter()
                .any(|field| field.label.contains("text") && field.value == "Inherited & shared")
        );
        assert!(
            source
                .iter()
                .any(|field| field.label.contains("fMaxHealth") && field.value == "200")
        );
        assert_eq!(fields(xml).unwrap().len(), 1);
        assert!(source_fields(b"<!DOCTYPE bad><Actor/>").is_err());
    }
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
    fn effectset_perk_values_preserve_variant_pipe_lists_and_strings() {
        let xml = br#"<effectSet name="MPerkBrawlerHighEffect" transferable="true" duration="-1"><effect class="GameProperty"><param name="value" value="0.17|0.19|0.23"/><param name="bUITreatAsPercentage" value="True"/><param name="bApplyToSelf" value="1"/></effect></effectSet>"#;
        let mut f = fields(xml).unwrap();
        assert_eq!(f.len(), 5, "string values are not editable numbers");
        assert_eq!(f[0].attribute, "transferable");
        assert_eq!(f[2].original, "0.17|0.19|0.23");
        assert_eq!(f[3].original, "True");
        f[2].value = "0.18|0.20|0.24".into();
        f[3].value = "False".into();
        let edited = apply(xml, &f).unwrap();
        let text = String::from_utf8(edited).unwrap();
        assert!(text.contains(r#"value="0.18|0.20|0.24""#));
        assert!(text.contains(r#"value="False""#));
        assert!(text.contains(r#"name="MPerkBrawlerHighEffect""#));
        // Stage counts are part of the authored structure and cannot drift.
        f[2].value = "0.18|0.20".into();
        assert!(apply(xml, &f).is_err());
    }
    #[test]
    fn cryxmlb_effectset_edits_preserve_binary_container_and_variants() {
        let xml = br#"<effectSet name="MPerkBrawlerHighEffect" levels="3"><effect class="GameProperty"><param name="value" value="0.17|0.19|0.23"/><param name="bUITreatAsPercentage" value="True"/></effect></effectSet>"#;
        let binary = crate::cryxml::encode_xml(xml, LIMIT).unwrap();
        let mut f = fields(&binary).unwrap();
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].original, "3");
        assert_eq!(f[1].original, "0.17|0.19|0.23");
        assert_eq!(f[2].original, "True");
        f[1].value = "0.18|0.20|0.24".into();
        f[2].value = "False".into();
        let edited = apply(&binary, &f).unwrap();
        assert!(edited.starts_with(b"CryXmlB\0"));
        let decoded = crate::cryxml::xml_bytes(&edited, LIMIT).unwrap();
        let text = String::from_utf8(decoded.into_owned()).unwrap();
        assert!(text.contains(r#"value="0.18|0.20|0.24""#));
        assert!(text.contains(r#"value="False""#));
        let unchanged = fields(&binary).unwrap();
        assert_eq!(
            apply(&binary, &unchanged).unwrap(),
            binary,
            "unchanged binary documents must keep their exact source bytes"
        );
    }
    #[test]
    #[ignore = "requires a local user-owned .effectset document"]
    fn real_effectset_document_round_trip() {
        let path =
            std::env::var("EVOLVE_EFFECTSET_DOCUMENT").expect("set EVOLVE_EFFECTSET_DOCUMENT");
        let bytes = std::fs::read(path).unwrap();
        let f = fields(&bytes).unwrap();
        assert!(!f.is_empty());
        if bytes.starts_with(b"CryXmlB\0") {
            crate::cryxml::xml_bytes(&bytes, LIMIT).unwrap();
        }
        let result = apply(&bytes, &f).unwrap();
        assert_eq!(
            result, bytes,
            "unchanged .effectset must stay byte-identical"
        );
        eprintln!("{} editable .effectset fields validated", f.len());
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
