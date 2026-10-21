//! Original E.M.E.T. attachment variants in one 3P CDF. No guessed meshes:
//! the UI can only select render attachments that the source CDF declares.
use crate::cdf::{AttachmentKind, CdfDefinition};
use crate::character_equipment::EquipmentOverrides;

pub const FACE: &str = "__emet_face";
pub const BEACON: &str = "__emet_respawn_beacon";
pub const NO_FACE: &str = "__none";
pub const BEACON_ON: &str = "show";
pub const BEACON_OFF: &str = "hide";

#[derive(Clone, Debug, Default)]
pub struct EmetOptions {
    pub faces: Vec<String>,
    pub beacons: Vec<String>,
}

pub fn is_emet_cdf(path: &str) -> bool {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    path.ends_with("/merc_emet/emet_3p.cdf")
}

fn tokenized(input: &str) -> Vec<String> {
    input
        .to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn is_face(name: &str, binding: &str) -> bool {
    let tokens = tokenized(name);
    let path = binding.replace('\\', "/").to_ascii_lowercase();
    // Only explicit expression identifiers or facial asset folders. A body,
    // helmet or whole-head mesh is NOT implicitly a substitute face.
    let named = [
        "oh",
        "smile",
        "happy",
        "sad",
        "angry",
        "blink",
        "grin",
        "frown",
        "surprised",
        "surprise",
        "shock",
        "scream",
        "laugh",
        "evil",
        "mad",
        "face",
        "expression",
    ];
    let face_folder = path.contains("/faces/")
        || path.contains("/expressions/")
        || path.contains("/expression/");
    let neutral = ["neutral", "normal", "default"];
    tokens.iter().any(|token| named.contains(&token.as_str()))
        || face_folder
        || (tokens.len() == 1 && neutral.contains(&tokens[0].as_str()))
        || (tokens.iter().any(|token| neutral.contains(&token.as_str()))
            && tokens.iter().any(|token| token == "face" || token == "expression"))
}

fn is_beacon(name: &str, binding: &str) -> bool {
    let a = name.to_ascii_lowercase();
    let b = binding.to_ascii_lowercase();
    a.contains("respawn") || a.contains("beacon") || b.contains("respawn") || b.contains("beacon")
}

/// Discover Emet 3P alternatives in the CDF itself; never search arbitrary
/// local assets or another hunter's folder for a face or beacon.
pub fn discover(path: &str, cdf: &CdfDefinition) -> Option<EmetOptions> {
    if !is_emet_cdf(path) {
        return None;
    }
    let mut result = EmetOptions::default();
    for item in &cdf.attachments {
        if !matches!(
            item.kind,
            AttachmentKind::Skin | AttachmentKind::UniqueSkin | AttachmentKind::Bone
        ) {
            continue;
        }
        if is_beacon(&item.name, &item.binding) {
            result.beacons.push(item.name.clone());
        } else if is_face(&item.name, &item.binding) {
            result.faces.push(item.name.clone());
        }
    }
    Some(result)
}

/// Apply only previously declared CDF attachment names; do not manufacture
/// face meshes, bone transforms or a manually placed Respawn Beacon.
/// Return an error when Emet-only options are applied to a different CDF.
pub fn choose<'a>(
    path: &str,
    cdf: &'a CdfDefinition,
    selected: Vec<&'a str>,
    overrides: &EquipmentOverrides,
) -> Result<Vec<&'a str>, String> {
    let face = overrides.get(FACE);
    let beacon = overrides.get(BEACON);
    if face.is_none() && beacon.is_none() && !is_emet_cdf(path) {
        return Ok(selected);
    }
    let options = discover(path, cdf).ok_or("Emet model options require its original 3P CDF")?;
    if let Some(face) = face
        && face != NO_FACE
        && !options.faces.iter().any(|name| name == face)
    {
        return Err("Face is not an authored Emet 3P CDF option".into());
    }
    if let Some(beacon) = beacon
        && (options.beacons.is_empty() || (beacon != BEACON_ON && beacon != BEACON_OFF))
    {
        return Err("Respawn Beacon is not an authored Emet 3P CDF option".into());
    }
    // Keep one expression. If no user choice exists, retain the first
    // declared expression, not every mutually exclusive face at once.
    let active_face = face
        .map(String::as_str)
        .unwrap_or_else(|| options.faces.first().map(String::as_str).unwrap_or(NO_FACE));
    let show_beacon = beacon.map(String::as_str).unwrap_or(BEACON_ON) == BEACON_ON;
    let result = selected
        .into_iter()
        .filter(|part| {
            if options.faces.iter().any(|name| name == part) {
                return *part == active_face;
            }
            if options.beacons.iter().any(|name| name == part) {
                return show_beacon;
            }
            true
        })
        .collect::<Vec<_>>();
    if result.is_empty() {
        return Err("Selected Emet options have no renderable CDF attachments".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdf::parse_cdf;

    fn emet() -> CdfDefinition {
        parse_cdf(br#"<CharacterDefinition>
            <Model File="characters/hunters/merc_emet/emet_3p_skeleton.chr"/>
            <AttachmentList>
            <Attachment AName="body" Type="CA_SKIN" Binding="characters/hunters/merc_emet/emet_body.skin"/>
            <Attachment AName="oh" Type="CA_SKIN" Binding="characters/hunters/merc_emet/oh.skin"/>
            <Attachment AName="smile" Type="CA_SKIN" Binding="characters/hunters/merc_emet/smile.skin"/>
            <Attachment AName="angry" Type="CA_SKIN" Binding="characters/hunters/merc_emet/angry.skin"/>
            <Attachment AName="respawn_beacon" Type="CA_BONE" BoneName="weapon_joint"
                Binding="characters/hunters/merc_emet/respawn_beacon.cgf"/>
            </AttachmentList></CharacterDefinition>"#).unwrap()
    }
    #[test]
    fn discovers_faces_and_beacon_from_original_emet_cdf() {
        let info = discover("Objects/characters/hunters/merc_emet/emet_3p.cdf", &emet()).unwrap();
        assert_eq!(info.faces, ["oh", "smile", "angry"]);
        assert_eq!(info.beacons, ["respawn_beacon"]);
        assert!(discover("characters/hunters/merc_tech/tech_3p.cdf", &emet()).is_none());
    }
    #[test]
    fn only_one_emet_face_can_be_visible_and_beacon_is_independent() {
        let cdf = emet();
        let path = "characters/hunters/merc_emet/emet_3p.cdf";
        let all = cdf.attachments.iter().map(|a| a.name.as_str()).collect();
        let original = choose(path, &cdf, all, &EquipmentOverrides::new()).unwrap();
        assert_eq!(original, ["body", "oh", "respawn_beacon"]);
        let mut options = EquipmentOverrides::new();
        options.insert(FACE.into(), "angry".into());
        options.insert(BEACON.into(), BEACON_OFF.into());
        let all = cdf.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            choose(path, &cdf, all, &options).unwrap(),
            ["body", "angry"]
        );
        options.insert(FACE.into(), NO_FACE.into());
        let all = cdf.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(choose(path, &cdf, all, &options).unwrap(), ["body"]);
    }
    #[test]
    fn generic_body_default_and_head_default_are_not_facial_expressions() {
        assert!(!is_face("body_default", "characters/hunters/merc_emet/emet_body.skin"));
        assert!(!is_face("head_default", "characters/hunters/merc_emet/emet_head.skin"));
        assert!(is_face("neutral", "characters/hunters/merc_emet/neutral.skin"));
        assert!(is_face("face_default", "characters/hunters/merc_emet/face_default.skin"));
    }
    #[test]
    fn refuses_invented_face_and_cross_character_options() {
        let cdf = emet();
        let mut options = EquipmentOverrides::new();
        options.insert(FACE.into(), "face_from_another_hunter".into());
        let all = cdf.attachments.iter().map(|a| a.name.as_str()).collect();
        assert!(
            choose(
                "characters/hunters/merc_emet/emet_3p.cdf",
                &cdf,
                all,
                &options
            )
            .is_err()
        );
        let all = cdf.attachments.iter().map(|a| a.name.as_str()).collect();
        assert!(
            choose(
                "characters/hunters/merc_tech/tech_3p.cdf",
                &cdf,
                all,
                &options
            )
            .is_err()
        );
    }
}
