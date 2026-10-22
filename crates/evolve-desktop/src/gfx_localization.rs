//! Exact-build CryEngine text_ui_*.xml lookup for *preview diagnostics*.
//! Never substitutes an English guess for an unavailable game string or
//! writes back to a PAK, movie, or imported Asset Library.
use evolve_core::{LibraryReferenceStatus, imported_library, resolve_imported_reference};
use quick_xml::{Reader, escape::unescape, events::Event};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

const MAX_MANIFEST: u64 = 32 * 1024 * 1024;
const MAX_XML: u64 = 4 * 1024 * 1024;
const MAX_FILES: usize = 24;
const MAX_ROW: usize = 128;
const MAX_MATCHES: usize = 256;

fn xml_name(value: &[u8]) -> &[u8] {
    value.rsplit(|c| *c == b':').next().unwrap_or(value)
}
fn regular(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > limit {
        return Err("Localization source is not a bounded regular file".into());
    }
    fs::read(path).map_err(|e| e.to_string())
}
fn label_key(label: &str) -> Option<String> {
    let key = label.strip_prefix('@')?.trim();
    if key.len() < 3
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return None;
    }
    Some(key.to_ascii_lowercase())
}
fn header_name(label: &str) -> String {
    label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}
fn consume_row(
    cells: &[String],
    columns: &mut Option<(usize, usize, Option<usize>)>,
    found: &mut HashMap<String, String>,
    wanted: &HashSet<String>,
) {
    if columns.is_none() {
        let headings = cells.iter().map(|v| header_name(v)).collect::<Vec<_>>();
        let key = headings.iter().position(|s| s == "KEY");
        let translation = headings.iter().position(|s| s == "TRANSLATEDTEXT");
        let original = headings.iter().position(|s| s == "ORIGINALTEXT");
        if let (Some(key), Some(text)) = (key, translation.or(original)) {
            *columns = Some((key, text, original));
        }
        return;
    }
    if let Some((key_index, value_index, original_index)) = *columns {
        let Some(key) = cells.get(key_index) else {
            return;
        };
        let cleaned = key.trim().trim_start_matches('@').to_ascii_lowercase();
        if !wanted.contains(&cleaned) {
            return;
        }
        let value = cells.get(value_index).map(|v| v.trim()).unwrap_or_default();
        let alternative = original_index
            .and_then(|i| cells.get(i))
            .map(|v| v.trim())
            .unwrap_or_default();
        let selected = if value.is_empty() { alternative } else { value };
        if !selected.is_empty() && selected.len() <= 1024 {
            found.insert(cleaned, selected.to_owned());
        }
    }
}

/// Read only the source's spreadsheet-style rows (KEY, TRANSLATED TEXT),
/// including namespaced SpreadsheetML Row/Cell/Data tags. Reject DTDs and
/// malformed XML; unknown formats remain unresolved, not guessed.
fn parse_sheet(bytes: &[u8], wanted: &HashSet<String>) -> Result<HashMap<String, String>, String> {
    if bytes.len() > MAX_XML as usize {
        return Err("Localization sheet too large".into());
    }
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut cells = Vec::<String>::new();
    let mut data = String::new();
    let mut in_row = false;
    let mut in_cell = false;
    let mut in_data = false;
    let mut columns = None;
    let mut found = HashMap::new();
    let mut rows = 0usize;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(event)) => match xml_name(event.name().as_ref()) {
                b"Row" => {
                    in_row = true;
                    cells.clear();
                    rows += 1;
                }
                b"Cell" if in_row => {
                    in_cell = true;
                    data.clear();
                }
                b"Data" if in_cell => {
                    in_data = true;
                }
                _ => {}
            },
            Ok(Event::Text(event)) if in_data => {
                let raw = String::from_utf8_lossy(event.as_ref());
                let decoded = unescape(&raw)
                    .map_err(|_| "Invalid localization XML escaping")?;
                if data.len() + decoded.len() > 2048 { return Err("Localization cell too long".into()); }
                data.push_str(&decoded);
            }
            Ok(Event::CData(event)) if in_data => {
                let decoded = String::from_utf8_lossy(event.as_ref());
                if data.len() + decoded.len() > 2048 {
                    return Err("Localization CDATA too long".into());
                }
                data.push_str(&decoded);
            }
            Ok(Event::End(event)) => match xml_name(event.name().as_ref()) {
                b"Data" => in_data = false,
                b"Cell" if in_row => {
                    if cells.len() >= MAX_ROW {
                        return Err("Localization row too wide".into());
                    }
                    cells.push(std::mem::take(&mut data));
                    in_cell = false;
                    in_data = false;
                }
                b"Row" if in_row => {
                    consume_row(&cells, &mut columns, &mut found, wanted);
                    in_row = false;
                }
                _ => {}
            },
            Ok(Event::DocType(_)) => return Err("Localization DTD is not permitted".into()),
            Ok(Event::Eof) => break,
            Err(_) => return Err("Malformed localization XML".into()),
            _ => {}
        }
        if rows > 250_000 || found.len() > MAX_MATCHES {
            return Err("Localization sheet exceeds supported limits".into());
        }
        buf.clear();
    }
    if columns.is_none() {
        return Err("No verified KEY/TRANSLATED TEXT column headers".into());
    }
    Ok(found)
}
fn relevant_path(member: &str, archive_name: &str, language: &str) -> bool {
    let lowered = member.to_ascii_lowercase().replace('\\', "/");
    let suffix = lowered.rsplit('/').next().unwrap_or("");
    let relevant = suffix.starts_with("text_ui_")
        && suffix.ends_with(".xml")
        && lowered.split('/').all(|p| p != ".." && p != ".");
    if !relevant {
        return false;
    }
    let prefix = format!("localization/{language}/");
    lowered.starts_with(&prefix)
        || (!lowered.contains('/')
            && archive_name.eq_ignore_ascii_case(&format!("{language}_xml.pak")))
}

/// Localizes *inspector labels only*. Ruffle/AVM text fields still require
/// verified native GFx translation and font initialization (not simulated).
pub fn menu_labels(root: Option<&Path>, build: Option<&str>, labels: &[String]) -> Value {
    let wanted: HashSet<String> = labels.iter().filter_map(|s| label_key(s)).collect();
    let mut status = "No verified build/language localization available".to_string();
    let mut sources = Vec::<String>::new();
    let mut translations = HashMap::<String, String>::new();
    let mut ambiguous = HashSet::<String>::new();
    if wanted.is_empty() {
        return json!({"language":"english","status":"No localization keys in UI_Data",
            "translations":{},"unresolved":[],"sources":[]});
    }
    if let (Some(root), Some(build)) = (root, build) {
        if !build.trim().is_empty() {
            match load_sheets(root, build, "english", &wanted) {
                Ok((values, files, conflicts)) => {
                    translations = values;
                    sources = files;
                    ambiguous = conflicts;
                    status = format!(
                        "{} verified English UI keys from {} exact-build localization sheets",
                        translations.len(),
                        sources.len()
                    );
                }
                Err(e) => status = format!("Localization unavailable: {e}"),
            }
        }
    }
    let unresolved = wanted
        .iter()
        .filter(|key| !translations.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    let mut unresolved = unresolved;
    unresolved.sort();
    let mut conflicts = ambiguous.into_iter().collect::<Vec<_>>();
    conflicts.sort();
    json!({"language":"english","status":status,"translations":translations,
        "unresolved":unresolved,"ambiguous":conflicts,"sources":sources})
}
fn load_sheets(
    root: &Path,
    build: &str,
    language: &str,
    wanted: &HashSet<String>,
) -> Result<(HashMap<String, String>, Vec<String>, HashSet<String>), String> {
    let library = imported_library(root).map_err(|e| e.to_string())?;
    let mut paths = HashSet::<String>::new();
    for archive in &library.archives {
        if !archive.builds.iter().any(|label| label == build) {
            continue;
        }
        let manifest_path = Path::new(&archive.directory).join("asset-library.json");
        let data = regular(&manifest_path, MAX_MANIFEST)?;
        let doc: Value = serde_json::from_slice(&data).map_err(|_| "Invalid library manifest")?;
        if doc["version"].as_u64() != Some(1) ||
            doc["name"].as_str() != Some(archive.name.as_str()) {
            return Err("Localization library manifest mismatch".into());
        }
        let Some(entries) = doc["entries"].as_array() else {
            return Err("Localization manifest missing entries".into());
        };
        for entry in entries {
            if let Some(name) = entry["path"].as_str() {
                if relevant_path(name, &archive.name, language) {
                    paths.insert(name.to_owned());
                }
            }
        }
    }
    if paths.len() > MAX_FILES {
        return Err("Too many same-build localization sheets".into());
    }
    let mut paths = paths.into_iter().collect::<Vec<_>>();
    paths.sort();
    let mut values = HashMap::<String, String>::new();
    let mut duplicates = HashSet::<String>::new();
    let mut sources = Vec::<String>::new();
    for path in paths {
        let LibraryReferenceStatus::Found(info) =
            resolve_imported_reference(root, &path, Some(build)).map_err(|e| e.to_string())?
        else {
            continue;
        };
        let bytes = regular(Path::new(&info.absolute_path), MAX_XML)?;
        let table = parse_sheet(&bytes, wanted)?;
        sources.push(format!("{}: {}", info.archive_name, path));
        for (key, value) in table {
            if duplicates.contains(&key) {
                continue;
            }
            if values.get(&key).is_some_and(|existing| existing != &value) {
                values.remove(&key);
                duplicates.insert(key);
            } else {
                values.insert(key, value);
            }
        }
    }
    Ok((values, sources, duplicates))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_source_spreadsheetml_and_preserves_untranslated_keys() {
        let xml = br#"<Workbook><Worksheet><Table>
            <Row><Cell><Data>KEY</Data></Cell><Cell><Data>ORIGINAL TEXT</Data></Cell><Cell><Data>TRANSLATED TEXT</Data></Cell></Row>
            <Row><Cell><Data>ui_STARTGAME</Data></Cell><Cell><Data>Start game</Data></Cell><Cell><Data>Play &amp; Continue</Data></Cell></Row>
            <Row><Cell><Data>ui_OTHER</Data></Cell><Cell><Data>Other</Data></Cell><Cell><Data></Data></Cell></Row>
            </Table></Worksheet></Workbook>"#;
        let wanted = ["ui_startgame".to_owned(), "ui_other".to_owned()]
            .into_iter()
            .collect();
        let found = parse_sheet(xml, &wanted).unwrap();
        assert_eq!(found["ui_startgame"], "Play & Continue");
        assert_eq!(found["ui_other"], "Other");
        assert!(!found.contains_key("unknown"));
    }
    #[test]
    fn refuses_doctype_and_unrelated_build_languages() {
        assert!(parse_sheet(b"<!DOCTYPE a><a/>", &HashSet::new()).is_err());
        assert!(relevant_path(
            "Localization/English/text_ui_menu.xml",
            "English_XML.pak",
            "english"
        ));
        assert!(relevant_path(
            "text_ui_menu.xml",
            "English_XML.pak",
            "english"
        ));
        assert!(!relevant_path(
            "Localization/German/text_ui_menu.xml",
            "English_XML.pak",
            "english"
        ));
        assert!(!relevant_path(
            "../text_ui_menu.xml",
            "English_XML.pak",
            "english"
        ));
        let report = menu_labels(None, None, &["@ui_STARTGAME".to_string()].to_vec());
        assert_eq!(report["unresolved"][0], "ui_startgame");
    }
}
