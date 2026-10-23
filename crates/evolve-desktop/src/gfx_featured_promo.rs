//! Guarded, undoable featured-promotion ActionScript presets for main_menu.gfx.
//! Only a private FFDec export is edited; no game, PAK or Library originals change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    All,
    Title,
    Try,
    Buy,
}
impl Default for Target {
    fn default() -> Self {
        Self::All
    }
}
impl Target {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Entire promotion",
            Self::Title => "Featured title",
            Self::Try => "TRY button",
            Self::Buy => "BUY button",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Title => "title",
            Self::Try => "try",
            Self::Buy => "buy",
        }
    }
    fn members(self) -> &'static [&'static str] {
        match self {
            Self::Title => &["mcFeatureTitle"],
            Self::Try => &["mcFeatureTry", "mcFeatureButtonTry"],
            Self::Buy => &["mcFeatureBuy", "mcFeatureButtonBuy"],
            Self::All => &[
                "mcFeatureTitle",
                "mcFeatureBuy",
                "mcFeatureTry",
                "mcFeatureBg",
                "mcFeatureFocus",
                "mcFeatureButtonBuy",
                "mcFeatureButtonTry",
                "mcHighlightFeaturedMount",
            ],
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Hide,
    Remove,
    Text,
}
fn quote(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.chars().count() > 160
        || value.trim() != value
        || value.chars().any(|c| c.is_control())
    {
        return Err("Enter 1–160 printable characters with no surrounding whitespace".into());
    }
    // Serialize a string literal, never interpolate user text as ActionScript.
    serde_json::to_string(value).map_err(|e| e.to_string())
}
fn actions(target: Target, mode: Mode, text: &str, prefix: &str) -> Result<String, String> {
    if mode == Mode::Text && target == Target::All {
        return Err("Choose the featured title, TRY or BUY before replacing text".into());
    }
    let value = if mode == Mode::Text {
        Some(quote(text)?)
    } else {
        if !text.is_empty() {
            return Err("Text is only valid for the text-edit action".into());
        }
        None
    };
    let mut out = String::new();
    for member in target.members() {
        out += &format!("{prefix}if (menu.{member} != undefined) {{\n");
        match mode {
            Mode::Hide | Mode::Remove => {
                out += &format!("{prefix}    menu.{member}._visible = false;\n");
                if mode == Mode::Remove {
                    out += &format!("{prefix}    menu.{member}.enabled = false;\n");
                    out += &format!(
                        "{prefix}    if (menu.{member}.text != undefined) menu.{member}.text = \"\";\n"
                    );
                    out += &format!(
                        "{prefix}    if (menu.{member}.removeMovieClip != undefined) menu.{member}.removeMovieClip();\n"
                    );
                }
            }
            Mode::Text => {
                let value = value.as_ref().unwrap();
                out += &format!("{prefix}    menu.{member}._visible = true;\n");
                out += &format!(
                    "{prefix}    if (menu.{member}.text != undefined) menu.{member}.text = {value};\n"
                );
                for field in [
                    "txtTitle",
                    "txtText",
                    "txtLabel",
                    "txtTry",
                    "txtBuy",
                    "label",
                    "textField",
                ] {
                    out += &format!(
                        "{prefix}    if (menu.{member}.{field} != undefined && menu.{member}.{field}.text != undefined) menu.{member}.{field}.text = {value};\n"
                    );
                }
            }
        }
        out += &format!("{prefix}}}\n");
    }
    Ok(out)
}
/// Recognize authored AVM1 function forms with ordinary whitespace between
/// tokens rather than relying on FFDec using one particular print format.
/// This is a candidate scan only; ambiguous definitions are rejected below.
fn definitions(source: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    for (at, _) in source.match_indices("setFeatured") {
        let before = source[..at].trim_end_matches(char::is_whitespace);
        let after = source[at + "setFeatured".len()..].trim_start_matches(char::is_whitespace);
        let named_function = before.ends_with("function")
            && before[..before.len() - "function".len()]
                .chars()
                .last()
                .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
            && after.starts_with('(');
        let assigned_function = after.strip_prefix('=')
            .map(str::trim_start)
            .and_then(|remaining| remaining.strip_prefix("function"))
            .is_some_and(|remaining| remaining.trim_start().starts_with('('));
        if named_function || assigned_function {
            offsets.push(at);
        }
    }
    offsets
}
pub fn has_definition(source: &str) -> bool {
    !definitions(source).is_empty()
}
/// Find exactly one authored setFeatured body, skipping quoted strings and comments.
fn blocks(source: &str) -> Result<(usize, usize), String> {
    let starts = definitions(source);
    if starts.len() != 1 {
        return Err(
            "Expected exactly one original setFeatured function; use FFDec manually".into(),
        );
    }
    let after = &source[starts[0]..];
    let open = starts[0] + after.find('{').ok_or("Missing original setFeatured body")?;
    let mut depth = 0usize;
    let mut state = 0u8; // 0 normal, 1 single quote, 2 double quote, 3 line, 4 block
    let bytes = source.as_bytes();
    let mut i = open;
    while i < bytes.len() {
        let c = bytes[i];
        let next = bytes.get(i + 1).copied().unwrap_or_default();
        if state == 3 {
            if c == b'\n' {
                state = 0;
            }
        } else if state == 4 {
            if c == b'*' && next == b'/' {
                state = 0;
                i += 1;
            }
        } else if state == 1 || state == 2 {
            if c == b'\\' {
                i += 1;
            } else if (state == 1 && c == b'\'') || (state == 2 && c == b'"') {
                state = 0;
            }
        } else if c == b'/' && next == b'/' {
            state = 3;
            i += 1;
        } else if c == b'/' && next == b'*' {
            state = 4;
            i += 1;
        } else if c == b'\'' {
            state = 1;
        } else if c == b'"' {
            state = 2;
        } else if c == b'{' {
            depth += 1;
        } else if c == b'}' {
            depth -= 1;
            if depth == 0 {
                return Ok((open, i));
            }
        }
        i += 1;
    }
    Err("Cannot identify end of original setFeatured function".into())
}
const MARKER: &str = "__evolveFeaturedPromoPreset_";
/// Patch a *single* exact original function. Guard belongs to an invisible
/// menu child and is installed by every new setFeatured instance; it checks
/// every display frame so future offers cannot re-show the old promotion.
pub fn patch(source: &str, mode: Mode, target: Target, text: &str) -> Result<String, String> {
    if source.len() > 64 * 1024 * 1024 {
        return Err("ActionScript source exceeds editor bounds".into());
    }
    let (_, end) = blocks(source)?;
    let marker = format!("{MARKER}{}", target.key());
    if source.contains(&marker) {
        return Err("A preset already edits this target; Undo before replacing it".into());
    }
    if target == Target::All {
        if ["title", "try", "buy"]
            .iter()
            .any(|key| source.contains(&format!("{MARKER}{key}")))
        {
            return Err("Individual promotion edits are staged; Undo before editing all".into());
        }
    } else if source.contains(&format!("{MARKER}all")) {
        return Err("Entire promotion already edited; Undo before targeting one control".into());
    }
    if source.contains("__evolveFeaturedPromoPreset =") {
        return Err("Unsupported earlier promotion patch detected; Undo first".into());
    }
    let (start, _) = blocks(source)?;
    // A previous guard contains an intentional early return for destroyed
    // menu instances. Only inspect the original function body, not an
    // independently staged preset's nested onEnterFrame handler.
    let body = &source[start..end];
    let original = &body[..body.find(&format!("var {MARKER}")).unwrap_or(body.len())];
    if original.contains("return;") {
        return Err(
            "Original setFeatured contains an early return; manual FFDec editing required".into(),
        );
    }
    let guard_name = format!("__evolveFeaturedPromoGuard_{}", target.key());
    let once = actions(target, mode, text, "        ")?;
    let repeat = actions(target, mode, text, "                ")?;
    let injection = format!(
        "\n    var {marker} = true;\n    {{\n        var menu = _root;\n{once}    }}\n    if (_root.{guard_name} == undefined && _root.createEmptyMovieClip != undefined && _root.getNextHighestDepth != undefined) {{\n        var guardClip = _root.createEmptyMovieClip(\"{guard_name}\", _root.getNextHighestDepth());\n        if (guardClip != undefined) {{\n            guardClip.onEnterFrame = function() {{\n                var menu = this._parent;\n                if (menu == undefined) return;\n{repeat}            }};\n        }}\n    }}\n"
    );
    let mut result = String::with_capacity(source.len() + injection.len());
    result.push_str(&source[..end]);
    result.push_str(&injection);
    result.push_str(&source[end..]);
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    const ORIGINAL: &str = r#"function setFeatured(info) {
    // Comment brace }
    var example = "{string}";
    _root.mcFeatureTitle.text = info.name;
}
"#;
    #[test]
    fn persistent_removal_for_all_instances() {
        let result = patch(ORIGINAL, Mode::Remove, Target::All, "").unwrap();
        assert!(result.contains("guardClip.onEnterFrame = function()"));
        assert!(result.contains("menu.mcFeatureTry._visible = false"));
        assert!(result.contains("menu.mcFeatureButtonBuy.removeMovieClip()"));
        assert!(!result.contains("_root.onEnterFrame ="));
        assert!(!result.contains("setInterval("));
        assert!(patch(&result, Mode::Remove, Target::All, "").is_err());
    }
    #[test]
    fn persistent_custom_text_and_independent_controls() {
        let a = patch(ORIGINAL, Mode::Text, Target::Title, "GLACIAL \"MOD\"").unwrap();
        assert!(a.contains("menu.mcFeatureTitle.text = \"GLACIAL \\\"MOD\\\"\""));
        assert!(a.contains("guardClip.onEnterFrame = function()"));
        let b = patch(&a, Mode::Remove, Target::Try, "").unwrap();
        assert!(b.contains("menu.mcFeatureTry.enabled = false"));
        assert!(!b.contains("menu.mcFeatureTitle.enabled = false"));
        assert!(patch(&b, Mode::Remove, Target::All, "").is_err());
    }
    #[test]
    fn accepts_ffdec_whitespace_in_featured_signature() {
        for candidate in [
            "function setFeatured (info) { _root.mcFeatureTitle.text = info; }",
            "setFeatured = function (info) { _root.mcFeatureTitle.text = info; };",
            "setFeatured=function(info) { _root.mcFeatureTitle.text = info; };",
        ] {
            assert!(has_definition(candidate));
            assert!(patch(candidate, Mode::Hide, Target::Title, "").is_ok());
        }
        assert!(!has_definition("function another() { setFeatured(); }"));
    }
    #[test]
    fn guards_invalid_and_ambiguous_scripts() {
        assert!(patch(ORIGINAL, Mode::Text, Target::All, "text").is_err());
        assert!(patch(ORIGINAL, Mode::Text, Target::Title, "bad\ntext").is_err());
        assert!(
            patch(
                &(ORIGINAL.to_owned() + ORIGINAL),
                Mode::Hide,
                Target::All,
                ""
            )
            .is_err()
        );
        assert!(
            patch(
                "function setFeatured() { if (x) return; }",
                Mode::Hide,
                Target::All,
                ""
            )
            .is_err()
        );
        assert!(patch("function setFeatured() {", Mode::Hide, Target::All, "").is_err());
    }
}
