//! Source-annotated CryEngine blendspace and combined-blendspace definitions.
//! Only explicit SetPara coordinates and authored Face simplices are evaluated.
//! Parameter extraction from root motion, pseudo examples, extrapolation,
//! locked transitions and game-specific combo-graph state machines are NOT
//! inferred from incomplete source metadata.
use crate::cryxml;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

const MAX_XML: usize = 2 * 1024 * 1024;
const MAX_EXAMPLES: usize = 256;
const MAX_FACES: usize = 1024;
const MAX_SPACES: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct BlendDimension {
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub choose_space: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlendExample {
    pub animation: String,
    pub coordinates: [Option<f32>; 3],
    pub playback_scale: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlendFace {
    pub vertices: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlendSpace {
    pub dimensions: Vec<BlendDimension>,
    pub examples: Vec<BlendExample>,
    pub faces: Vec<BlendFace>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CombinedSpace {
    pub dimensions: Vec<BlendDimension>,
    pub children: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlendDocument {
    Space(BlendSpace),
    Combined(CombinedSpace),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlendWeight {
    pub example: usize,
    pub weight: f32,
}

fn invalid(reason: impl Into<String>) -> String {
    format!(
        "Unsupported or invalid authored blendspace: {}",
        reason.into()
    )
}

fn attributes(tag: &BytesStart<'_>) -> Result<Vec<(String, String)>, String> {
    let mut output = Vec::new();
    for item in tag.attributes() {
        let item = item.map_err(|error| invalid(error.to_string()))?;
        let name = std::str::from_utf8(item.key.as_ref())
            .map_err(|_| invalid("non-UTF8 XML attribute"))?
            .to_ascii_lowercase();
        let value = item
            .unescape_value()
            .map_err(|e| invalid(e.to_string()))?
            .into_owned();
        if output.iter().any(|(existing, _)| existing == &name) {
            return Err(invalid("duplicate XML attribute"));
        }
        output.push((name, value));
    }
    Ok(output)
}

fn field<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn number(value: &str) -> Result<f32, String> {
    let parsed = value
        .parse::<f32>()
        .map_err(|_| invalid("invalid numeric coordinate"))?;
    if !parsed.is_finite() {
        return Err(invalid("non-finite numeric coordinate"));
    }
    Ok(parsed)
}

fn safe_animation_path(value: &str) -> bool {
    let path = value.replace('\\', "/");
    !path.is_empty()
        && path.len() <= 512
        && !path.starts_with('/')
        && !path.contains(':')
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        && path.chars().all(|ch| !ch.is_control())
}

fn parse_dimension(attrs: &[(String, String)], combined: bool) -> Result<BlendDimension, String> {
    let name = field(attrs, "name").ok_or_else(|| invalid("dimension has no name"))?;
    if name.is_empty() || name.len() > 64 {
        return Err(invalid("invalid dimension name"));
    }
    let choose_space = field(attrs, "chooseblendspace") == Some("1");
    if !combined && choose_space {
        return Err(invalid("ChooseBlendSpace only belongs to .comb"));
    }
    let min = if let Some(v) = field(attrs, "min") {
        number(v)?
    } else if combined {
        0.0
    } else {
        return Err(invalid("missing minimum"));
    };
    let max = if let Some(v) = field(attrs, "max") {
        number(v)?
    } else if combined {
        0.0
    } else {
        return Err(invalid("missing maximum"));
    };
    if !combined && min >= max {
        return Err(invalid("dimension min must precede max"));
    }
    Ok(BlendDimension {
        name: name.to_owned(),
        min,
        max,
        choose_space,
    })
}

fn parse_example(attrs: &[(String, String)]) -> Result<BlendExample, String> {
    let animation = field(attrs, "aname").ok_or_else(|| invalid("example has no AName"))?;
    if animation.is_empty() || animation.len() > 512 || animation.chars().any(|c| c.is_control()) {
        return Err(invalid("invalid source animation name"));
    }
    let mut coordinates = [None; 3];
    for (i, slot) in coordinates.iter_mut().enumerate() {
        if let Some(v) = field(attrs, &format!("setpara{i}")) {
            *slot = Some(number(v)?);
        }
    }
    let playback_scale = field(attrs, "playbackscale").map_or(Ok(1.0), number)?;
    if !(0.01..=8.0).contains(&playback_scale) {
        return Err(invalid("unsupported playback scale"));
    }
    Ok(BlendExample {
        animation: animation.to_owned(),
        coordinates,
        playback_scale,
    })
}

fn parse_face(attrs: &[(String, String)]) -> Result<BlendFace, String> {
    let mut vertices = Vec::new();
    for i in 0..4 {
        let Some(value) = field(attrs, &format!("p{i}")) else {
            break;
        };
        vertices.push(
            value
                .parse::<usize>()
                .map_err(|_| invalid("invalid Face vertex index"))?,
        );
    }
    if vertices.len() < 2 || (vertices.len()..4).any(|i| field(attrs, &format!("p{i}")).is_some()) {
        return Err(invalid("face needs contiguous p0..pN vertices"));
    }
    let mut sorted = vertices.clone();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() != vertices.len() {
        return Err(invalid("face repeats a source example"));
    }
    Ok(BlendFace { vertices })
}

/// Decode only XML elements/attributes explicitly authored for a blendspace.
/// The engine's inferred motion annotations are intentionally not synthesized.
pub fn parse_blend_document(bytes: &[u8]) -> Result<BlendDocument, String> {
    if bytes.is_empty() || bytes.len() > MAX_XML {
        return Err(invalid("source exceeds bounded XML size"));
    }
    let decoded = cryxml::xml_bytes(bytes, MAX_XML).map_err(|e| invalid(e.to_string()))?;
    let text = decoded.as_ref();
    let mut reader = Reader::from_reader(text);
    let mut parents: Vec<String> = Vec::new();
    let mut kind: Option<bool> = None; // true: combined
    let mut dimensions = Vec::new();
    let mut examples = Vec::new();
    let mut faces = Vec::new();
    let mut children = Vec::new();
    loop {
        let event = reader.read_event().map_err(|e| invalid(e.to_string()))?;
        match event {
            Event::Start(ref tag) | Event::Empty(ref tag) => {
                let name = std::str::from_utf8(tag.name().as_ref())
                    .map_err(|_| invalid("bad tag"))?
                    .to_ascii_lowercase();
                let parent = parents.last().map(String::as_str);
                if parents.is_empty() {
                    kind = match (kind, name.as_str()) {
                        (None, "paragroup") => Some(false),
                        (None, "combinedblendspace") => Some(true),
                        _ => return Err(invalid("expected ParaGroup or CombinedBlendSpace root")),
                    };
                } else {
                    let attrs = attributes(tag)?;
                    match (name.as_str(), parent) {
                        ("param", Some("dimensions")) => {
                            if dimensions.len() >= 3 {
                                return Err(invalid("only 1D–3D supported"));
                            }
                            let dimension = parse_dimension(&attrs, kind.unwrap_or(false))?;
                            if dimensions.iter().any(|d: &BlendDimension| {
                                d.name.eq_ignore_ascii_case(&dimension.name)
                            }) {
                                return Err(invalid("repeated parameter name"));
                            }
                            dimensions.push(dimension);
                        }
                        ("example", Some("examplelist")) if kind == Some(false) => {
                            if examples.len() >= MAX_EXAMPLES {
                                return Err(invalid("too many examples"));
                            }
                            examples.push(parse_example(&attrs)?);
                        }
                        ("face", Some("blendable")) if kind == Some(false) => {
                            if faces.len() >= MAX_FACES {
                                return Err(invalid("too many faces"));
                            }
                            faces.push(parse_face(&attrs)?);
                        }
                        ("blendspace", Some("blendspaces")) if kind == Some(true) => {
                            if children.len() >= MAX_SPACES {
                                return Err(invalid("too many combined spaces"));
                            }
                            let path = field(&attrs, "aname")
                                .ok_or_else(|| invalid("combined child has no AName"))?;
                            if !safe_animation_path(path)
                                || !path.to_ascii_lowercase().ends_with(".bspace")
                            {
                                return Err(invalid("unsafe or unsupported combined child path"));
                            }
                            children.push(path.replace('\\', "/"));
                        }
                        _ => {}
                    }
                }
                if matches!(event, Event::Start(_)) {
                    if parents.len() >= 32 {
                        return Err(invalid("XML too deeply nested"));
                    }
                    parents.push(name);
                }
            }
            Event::End(tag) => {
                let name = std::str::from_utf8(tag.name().as_ref())
                    .map_err(|_| invalid("bad closing tag"))?
                    .to_ascii_lowercase();
                if parents.pop().as_deref() != Some(name.as_str()) {
                    return Err(invalid("mismatched closing tag"));
                }
            }
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err(invalid("XML external entities"));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !parents.is_empty() {
        return Err(invalid("truncated XML"));
    }
    let combined = kind.ok_or_else(|| invalid("missing root"))?;
    if combined {
        if children.is_empty()
            || dimensions.is_empty()
            || !dimensions.iter().any(|d: &BlendDimension| d.choose_space)
        {
            return Err(invalid(
                "combined spaces require child paths and ChooseBlendSpace parameter",
            ));
        }
        Ok(BlendDocument::Combined(CombinedSpace {
            dimensions,
            children,
        }))
    } else {
        let d = dimensions.len();
        if !(1..=3).contains(&d) || examples.is_empty() || faces.is_empty() {
            return Err(invalid("missing dimensions, examples, or annotated faces"));
        }
        for example in &examples {
            if (0..d).any(|i| example.coordinates[i].is_none()) {
                return Err(invalid(
                    "unannotated examples need root-motion extraction; not yet supported",
                ));
            }
        }
        for face in &faces {
            if face.vertices.len() != d + 1
                || face.vertices.iter().any(|&vertex| vertex >= examples.len())
            {
                return Err(invalid("face dimensionality or vertex out of bounds"));
            }
        }
        Ok(BlendDocument::Space(BlendSpace {
            dimensions,
            examples,
            faces,
        }))
    }
}

fn simplex_weights(coords: &[[f32; 3]], values: &[f32]) -> Option<Vec<f32>> {
    let d = values.len();
    if coords.len() != d + 1 || d == 0 || d > 3 {
        return None;
    }
    let mut matrix = [[0.0_f64; 5]; 4];
    for axis in 0..d {
        for (col, point) in coords.iter().enumerate() {
            matrix[axis][col] = f64::from(point[axis]);
        }
        matrix[axis][d + 1] = f64::from(values[axis]);
    }
    for item in matrix[d].iter_mut().take(d + 1) {
        *item = 1.0;
    }
    matrix[d][d + 1] = 1.0;
    let n = d + 1;
    for col in 0..n {
        let pivot =
            (col..n).max_by(|&a, &b| matrix[a][col].abs().total_cmp(&matrix[b][col].abs()))?;
        if matrix[pivot][col].abs() < 1e-9 {
            return None;
        }
        matrix.swap(col, pivot);
        let inverse = 1.0 / matrix[col][col];
        for value in matrix[col].iter_mut().take(n + 1).skip(col) {
            *value *= inverse;
        }
        let pivot_row = matrix[col];
        for (row_index, row) in matrix.iter_mut().take(n).enumerate() {
            if row_index == col {
                continue;
            }
            let factor = row[col];
            for (item, value) in row.iter_mut().take(n + 1).enumerate().skip(col) {
                *value -= factor * pivot_row[item];
            }
        }
    }
    let weights = (0..n).map(|row| matrix[row][n]).collect::<Vec<_>>();
    if weights.iter().any(|&v| !(-1e-5..=1.00001).contains(&v)) {
        return None;
    }
    let total: f64 = weights.iter().map(|v| v.max(0.0)).sum();
    if total <= 1e-9 {
        return None;
    }
    Some(
        weights
            .iter()
            .map(|&v| (v.max(0.0) / total) as f32)
            .collect(),
    )
}

/// Return only authored vertices in a containing face (segment / triangle /
/// tetrahedron). No nearest-neighbor guesses, extrapolation or silent fallbacks.
pub fn authored_blend_weights(
    space: &BlendSpace,
    values: &[f32],
) -> Result<Vec<BlendWeight>, String> {
    if values.len() != space.dimensions.len() || values.iter().any(|v| !v.is_finite()) {
        return Err(invalid("invalid parameter vector"));
    }
    for (dimension, &value) in space.dimensions.iter().zip(values) {
        if !(dimension.min..=dimension.max).contains(&value) {
            return Err(invalid(format!(
                "{} outside authored range",
                dimension.name
            )));
        }
    }
    for face in &space.faces {
        let coords = face
            .vertices
            .iter()
            .map(|&index| {
                let example = &space.examples[index];
                std::array::from_fn(|i| example.coordinates[i].unwrap_or(0.0))
            })
            .collect::<Vec<_>>();
        if let Some(weights) = simplex_weights(&coords, values) {
            return Ok(face
                .vertices
                .iter()
                .zip(weights)
                .filter(|(_, weight)| *weight > 1e-6)
                .map(|(&example, weight)| BlendWeight { example, weight })
                .collect());
        }
    }
    Err(invalid(
        "input is outside all authored blend faces or the face is degenerate",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(xml: &str) -> BlendSpace {
        match parse_blend_document(xml.as_bytes()).unwrap() {
            BlendDocument::Space(space) => space,
            _ => panic!("expected bspace"),
        }
    }
    #[test]
    fn one_dimensional_segments_interpolate_and_reject_extrapolation() {
        let s = parse(
            r#"<ParaGroup><Dimensions><Param name="BlendWeight" min="0" max="1"/></Dimensions><ExampleList><Example AName="walk" SetPara0="0"/><Example AName="run" SetPara0="1"/></ExampleList><Blendable><Face p0="0" p1="1"/></Blendable></ParaGroup>"#,
        );
        let w = authored_blend_weights(&s, &[0.25]).unwrap();
        assert_eq!(
            w,
            vec![
                BlendWeight {
                    example: 0,
                    weight: 0.75
                },
                BlendWeight {
                    example: 1,
                    weight: 0.25
                }
            ]
        );
        assert!(authored_blend_weights(&s, &[1.2]).is_err());
    }
    #[test]
    fn two_dimensional_triangle_respects_simplex_weights() {
        let s = parse(
            r#"<ParaGroup><Dimensions><Param name="MoveSpeed" min="0" max="1"/><Param name="TurnSpeed" min="0" max="1"/></Dimensions><ExampleList><Example AName="idle" SetPara0="0" SetPara1="0"/><Example AName="walk" SetPara0="1" SetPara1="0"/><Example AName="turn" SetPara0="0" SetPara1="1"/></ExampleList><Blendable><Face p0="0" p1="1" p2="2"/></Blendable></ParaGroup>"#,
        );
        let w = authored_blend_weights(&s, &[0.25, 0.25]).unwrap();
        assert_eq!(
            w,
            vec![
                BlendWeight {
                    example: 0,
                    weight: 0.5
                },
                BlendWeight {
                    example: 1,
                    weight: 0.25
                },
                BlendWeight {
                    example: 2,
                    weight: 0.25
                }
            ]
        );
        assert!(authored_blend_weights(&s, &[1.0, 1.0]).is_err());
    }
    #[test]
    fn three_dimensional_tetrahedron_blends_slope() {
        let s = parse(
            r#"<ParaGroup><Dimensions><Param name="MoveSpeed" min="0" max="1"/><Param name="TurnSpeed" min="0" max="1"/><Param name="TravelSlope" min="0" max="1"/></Dimensions><ExampleList><Example AName="a" SetPara0="0" SetPara1="0" SetPara2="0"/><Example AName="b" SetPara0="1" SetPara1="0" SetPara2="0"/><Example AName="c" SetPara0="0" SetPara1="1" SetPara2="0"/><Example AName="d" SetPara0="0" SetPara1="0" SetPara2="1"/></ExampleList><Blendable><Face p0="0" p1="1" p2="2" p3="3"/></Blendable></ParaGroup>"#,
        );
        let w = authored_blend_weights(&s, &[0.25, 0.25, 0.25]).unwrap();
        assert_eq!(w.len(), 4);
        assert!(w.iter().all(|item| (item.weight - 0.25).abs() < 1e-5));
        assert!(authored_blend_weights(&s, &[0.5, 0.5, 0.5]).is_err());
    }
    #[test]
    fn combined_blendspace_parses_only_safe_child_paths() {
        let xml = r#"<CombinedBlendSpace><Dimensions><Param name="TurnAngle" ChooseBlendSpace="1"/></Dimensions><BlendSpaces><BlendSpace AName="animations/left.bspace"/><BlendSpace AName="animations/right.bspace"/></BlendSpaces></CombinedBlendSpace>"#;
        let BlendDocument::Combined(comb) = parse_blend_document(xml.as_bytes()).unwrap() else {
            panic!("combined")
        };
        assert_eq!(comb.children.len(), 2);
        assert!(
            parse_blend_document(
                xml.replace("animations/right.bspace", "../right.bspace")
                    .as_bytes()
            )
            .is_err()
        );
    }
    #[test]
    fn unannotated_or_degenerate_data_does_not_claim_native_interpolation() {
        let xml = r#"<ParaGroup><Dimensions><Param name="MoveSpeed" min="0" max="1"/></Dimensions><ExampleList><Example AName="walk"/><Example AName="run"/></ExampleList><Blendable><Face p0="0" p1="1"/></Blendable></ParaGroup>"#;
        assert!(parse_blend_document(xml.as_bytes()).is_err());
        let bad = xml
            .replace("AName=\"walk\"", "AName=\"walk\" SetPara0=\"0\"")
            .replace("AName=\"run\"", "AName=\"run\" SetPara0=\"0\"");
        let space = parse(&bad);
        assert!(authored_blend_weights(&space, &[0.0]).is_err());
    }
}
