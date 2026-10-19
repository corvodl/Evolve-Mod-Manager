//! Synthetic documents encoded by the reference manager's CryEngine 3
//! encoder. No game assets, credentials or signing material are included.
use evolve_core::cryxml::xml_bytes;
use evolve_core::{animation_references, parse_cdf, parse_level_scene, parse_material};

const CHARACTER: &[u8] = include_bytes!("fixtures/cryxml/character.bin");
const MATERIAL: &[u8] = include_bytes!("fixtures/cryxml/material.bin");
const LEVEL: &[u8] = include_bytes!("fixtures/cryxml/level.bin");
const ANIMATIONS: &[u8] = include_bytes!("fixtures/cryxml/animations.bin");

#[test]
fn reference_binary_and_text_documents_have_identical_reader_results() {
    assert_eq!(
        parse_cdf(CHARACTER).unwrap(),
        parse_cdf(include_bytes!("fixtures/cryxml/character.xml")).unwrap()
    );
    let mat = parse_material(MATERIAL).unwrap();
    assert_eq!(mat.slots[0].name, "Test & Fixture");
    assert_eq!(mat.slots[0].maps[0].path, "Characters/body.tif");
    let refs = animation_references(ANIMATIONS).unwrap();
    assert_eq!(refs[0].path, "Animations/test/idle.caf");
    let scene = parse_level_scene(&[("level", LEVEL)]).unwrap();
    assert_eq!(scene.objects[1].world.position, [7., 10., 13.]);
}

#[test]
fn escaping_preserves_attribute_and_text_values() {
    let xml = xml_bytes(include_bytes!("fixtures/cryxml/escaped.bin"), 4096).unwrap();
    let xml = std::str::from_utf8(&xml).unwrap();
    assert!(xml.contains("&quot;&lt;&amp;&apos;"));
    assert!(xml.contains("a &amp; b &lt; c"));
}

#[test]
fn every_truncation_and_outside_table_is_rejected() {
    for size in 8..CHARACTER.len() {
        assert!(
            xml_bytes(&CHARACTER[..size], 4096).is_err(),
            "accepted truncation at {size}"
        );
    }
    for offset in [12, 20, 28, 36, 44] {
        let mut bad = CHARACTER.to_vec();
        bad[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            xml_bytes(&bad, 4096).is_err(),
            "accepted bad offset at {offset}"
        );
    }
}

#[test]
fn rejects_parent_mismatch_overlap_and_expansion() {
    let mut bad = CHARACTER.to_vec();
    // Second node's parent no longer agrees with root's child list.
    bad[44 + 28 + 12..44 + 28 + 16].copy_from_slice(&3u32.to_le_bytes());
    assert!(xml_bytes(&bad, 4096).is_err());
    let mut bad = CHARACTER.to_vec();
    bad[20..24].copy_from_slice(&44u32.to_le_bytes());
    assert!(xml_bytes(&bad, 4096).is_err());
    assert!(xml_bytes(CHARACTER, 64).is_err());
}

#[test]
fn binary_level_can_be_edited_to_new_xml_without_changing_source() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.lyr");
    let output = temp.path().join("edited.lyr");
    std::fs::write(&source, LEVEL).unwrap();
    evolve_core::export_edited_layer(
        &source,
        &output,
        "a",
        evolve_core::Transform {
            position: [1., 2., 3.],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), LEVEL);
    assert_eq!(
        evolve_core::load_level_scene(&output).unwrap().objects[0]
            .world
            .position,
        [1., 2., 3.]
    );
    assert!(evolve_core::export_edited_layer(&source, &output, "a", Default::default()).is_err());
}
