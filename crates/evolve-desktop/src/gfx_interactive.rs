//! Experimental interactive GFX preview using an independently installed Ruffle
//! desktop runtime. The original ActionScript bytecode remains in the movie;
//! only supported Scaleform DDS image tags are reconstructed for rendering.
//! This is intentionally NOT a CryEngine host callback implementation.
use crate::{gfx_editor::Session, gfx_external_textures, gfx_live_web};
use evolve_core::gfx::{MAX_DECOMPRESSED, MAX_SOURCE, inspect_gfx};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

// Retained for the isolated desktop fallback and parser regression tests.
#[allow(dead_code)]
const MAX_PLAYER_BYTES: u64 = 350 * 1024 * 1024;
const MAX_IMPORTS: usize = 128;

/// Only UI-local ActionScript functions from the original main_menu.gfx
/// may be selected. Game-service callbacks remain inaccessible.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PreviewPreset {
    #[default]
    AsAuthored,
    OpenScreen,
    ShowEngagement,
    HideEngagement,
    ShowSigningIn,
    HideSigningIn,
    Back,
    ReturnToMainMenu,
    ShowPatchNotes,
    HidePatchNotes,
    ShowNews,
    HideNews,
    ShowChallenges,
    HideChallenges,
}

impl PreviewPreset {
    pub(crate) fn script_call(self) -> Option<(&'static str, Option<bool>)> {
        match self {
            Self::AsAuthored => None,
            Self::OpenScreen => Some(("openScreen", None)),
            Self::ShowEngagement => Some(("showEngagementPrompt", Some(true))),
            Self::HideEngagement => Some(("showEngagementPrompt", Some(false))),
            Self::ShowSigningIn => Some(("showSigningInPrompt", Some(true))),
            Self::HideSigningIn => Some(("showSigningInPrompt", Some(false))),
            Self::Back => Some(("goBack", None)),
            Self::ReturnToMainMenu => Some(("transitionToMainMenu", None)),
            Self::ShowPatchNotes => Some(("showPatchNotes", None)),
            Self::HidePatchNotes => Some(("hidePatchNotes", None)),
            Self::ShowNews => Some(("setNewsVisibility", Some(true))),
            Self::HideNews => Some(("setNewsVisibility", Some(false))),
            Self::ShowChallenges => Some(("setShowChallengesFlag", Some(true))),
            Self::HideChallenges => Some(("setShowChallengesFlag", Some(false))),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::AsAuthored => "Unmodified movie",
            Self::OpenScreen => "Open original main menu screen",
            Self::ShowEngagement => "Show engagement prompt",
            Self::HideEngagement => "Dismiss engagement prompt",
            Self::ShowSigningIn => "Show sign-in prompt",
            Self::HideSigningIn => "Dismiss sign-in prompt",
            Self::Back => "Go back (original menu handler)",
            Self::ReturnToMainMenu => "Return to main menu",
            Self::ShowPatchNotes => "Show patch notes",
            Self::HidePatchNotes => "Dismiss patch notes",
            Self::ShowNews => "Show news panel",
            Self::HideNews => "Hide news panel",
            Self::ShowChallenges => "Enable challenge controls",
            Self::HideChallenges => "Disable challenge controls",
        }
    }
}

/// Inspect the actual top-level AVM1 DefineFunction/DefineFunction2 opcode,
/// not an arbitrary text occurrence in a constant pool or embedded resource.
/// Reject unknown or conflicting signatures rather than guessing arguments.
fn defined_function_arity(actions: &[u8], name: &str) -> Option<usize> {
    let mut offset = 0;
    let mut arity = None;
    while offset < actions.len() {
        let opcode = actions[offset];
        offset += 1;
        if opcode == 0 {
            break;
        }
        let length = if opcode >= 0x80 {
            if offset + 2 > actions.len() {
                return None;
            }
            let count = usize::from(u16::from_le_bytes([actions[offset], actions[offset + 1]]));
            offset += 2;
            count
        } else {
            0
        };
        if length > actions.len() - offset {
            return None;
        }
        let payload = &actions[offset..offset + length];
        offset += length;
        if opcode != 0x9b && opcode != 0x8e {
            continue;
        }
        // AVM1 function action headers exclude their enclosed instruction
        // bodies. Skip the body to avoid accepting a nested function as a
        // global menu handler.
        if length < 2 {
            return None;
        }
        let body_length = usize::from(u16::from_le_bytes([
            payload[length - 2],
            payload[length - 1],
        ]));
        if body_length > actions.len() - offset {
            return None;
        }
        offset += body_length;
        let end = payload.iter().position(|b| *b == 0)?;
        if &payload[..end] != name.as_bytes() {
            continue;
        }
        if end + 3 > payload.len() {
            return None;
        }
        let count = usize::from(u16::from_le_bytes([payload[end + 1], payload[end + 2]]));
        if count > 16 {
            return None;
        }
        if arity.replace(count).is_some_and(|old| old != count) {
            return None;
        }
    }
    arity
}

/// Instrument only original DefineButton2 condition actions in the isolated
/// uncompressed preview movie. This reports the ACTUAL SWF button ID/condition
/// when Ruffle executes it; it does not guess a CryEngine command or intercept
/// native services. A trace precedes each original action stream without
/// changing its internal relative branch offsets.
fn trace_original_button_actions(swf: &[u8]) -> Result<(Vec<u8>, usize), String> {
    if swf.len() < 24 || swf.len() > MAX_DECOMPRESSED || &swf[..3] != b"FWS" {
        return Err("Button tracing requires a checked uncompressed SWF".into());
    }
    if u32::from_le_bytes(swf[4..8].try_into().unwrap()) as usize != swf.len() {
        return Err("Button tracing found a mismatched SWF length".into());
    }
    let nbits = usize::from(swf[8] >> 3);
    if !(1..=31).contains(&nbits) {
        return Err("Button tracing found an invalid frame RECT".into());
    }
    let start = 8 + (5 + 4 * nbits).div_ceil(8) + 4;
    if start > swf.len() {
        return Err("Button tracing found a truncated frame header".into());
    }
    let mut output = swf[..start].to_vec();
    let mut cursor = start;
    let mut tag_count = 0usize;
    let mut action_count = 0usize;
    while cursor + 2 <= swf.len() {
        let tag_start = cursor;
        let header = u16::from_le_bytes(swf[cursor..cursor + 2].try_into().unwrap());
        cursor += 2;
        let code = header >> 6;
        let mut length = usize::from(header & 63);
        if length == 63 {
            if cursor + 4 > swf.len() {
                return Err("Truncated SWF tag during button tracing".into());
            }
            length = u32::from_le_bytes(swf[cursor..cursor + 4].try_into().unwrap()) as usize;
            cursor += 4;
        }
        if length > swf.len() - cursor || tag_count >= 80_000 {
            return Err("Invalid or excessive SWF tags during button tracing".into());
        }
        let tag = &swf[cursor..cursor + length];
        let tag_end = cursor + length;
        cursor = tag_end;
        tag_count += 1;
        if code == 34 {
            if tag.len() < 5 {
                return Err("Truncated original DefineButton2 tag".into());
            }
            let offset = usize::from(u16::from_le_bytes([tag[3], tag[4]]));
            if offset == 0 {
                output.extend_from_slice(&swf[tag_start..tag_end]);
                continue;
            }
            let action_start = 3usize.saturating_add(offset);
            if action_start + 4 > tag.len() {
                return Err("Invalid original DefineButton2 action offset".into());
            }
            let button_id = u16::from_le_bytes([tag[0], tag[1]]);
            let mut modified = tag[..action_start].to_vec();
            let mut pos = action_start;
            loop {
                if pos + 4 > tag.len() || action_count >= 1024 {
                    return Err("Invalid or excessive original button conditions".into());
                }
                let declared = usize::from(u16::from_le_bytes([tag[pos], tag[pos + 1]]));
                let end = if declared == 0 {
                    tag.len()
                } else {
                    pos + declared
                };
                if end > tag.len() || end <= pos + 4 {
                    return Err("Truncated original button condition actions".into());
                }
                let condition = u16::from_le_bytes([tag[pos + 2], tag[pos + 3]]);
                let message = format!("evolve-button:{button_id}:{condition}");
                let mut trace = Vec::new();
                avm1_push_string(&mut trace, &message)?;
                trace.push(0x26); // ActionTrace consumes the pushed original ID.
                let new_size = if declared == 0 {
                    0
                } else {
                    declared + trace.len()
                };
                if new_size > usize::from(u16::MAX) {
                    return Err("Traced button condition exceeds SWF limits".into());
                }
                modified.extend_from_slice(&(new_size as u16).to_le_bytes());
                modified.extend_from_slice(&tag[pos + 2..pos + 4]);
                modified.extend_from_slice(&trace);
                modified.extend_from_slice(&tag[pos + 4..end]);
                action_count += 1;
                pos = end;
                if declared == 0 {
                    break;
                }
                if pos == tag.len() {
                    return Err("Non-final original button condition has no terminator".into());
                }
            }
            output.extend_from_slice(&((34u16 << 6) | 63).to_le_bytes());
            output.extend_from_slice(&(modified.len() as u32).to_le_bytes());
            output.extend_from_slice(&modified);
        } else {
            output.extend_from_slice(&swf[tag_start..tag_end]);
        }
        if output.len() > MAX_DECOMPRESSED {
            return Err("Button diagnostics exceed isolated SWF size limit".into());
        }
        if code == 0 {
            break;
        }
    }
    if cursor != swf.len() {
        return Err("Trailing or incomplete SWF tags during button tracing".into());
    }
    let final_length = output.len() as u32;
    output[4..8].copy_from_slice(&final_length.to_le_bytes());
    Ok((output, action_count))
}

/// Register an allowlisted bridge on the original *first* frame, after the
/// movie has declared its root functions. Unlike the old one-shot patch, the
/// callback is invoked later through Ruffle Web's ExternalInterface in a
/// persistent player. No native CryEngine interfaces are implemented.
///
/// The wrapper returns `true` after calling the original handler so the
/// JavaScript caller can distinguish dispatched actions from missing callbacks.
/// This is NOT proof that a handler could complete a native game transition.
pub(crate) fn install_live_callbacks(
    swf: &[u8],
    virtual_movie: &str,
) -> Result<(Vec<u8>, Vec<PreviewPreset>), String> {
    if !virtual_movie.eq_ignore_ascii_case("Libs/UI/main_menu.gfx") {
        return Ok((swf.to_vec(), Vec::new()));
    }
    if swf.len() < 24 || swf.len() > MAX_DECOMPRESSED || &swf[..3] != b"FWS" {
        return Err(
            "Live ActionScript callbacks need a validated uncompressed main-menu SWF".into(),
        );
    }
    if usize::try_from(u32::from_le_bytes(swf[4..8].try_into().unwrap())).ok() != Some(swf.len()) {
        return Err("Live callback SWF length does not match its header".into());
    }
    let nbits = usize::from(swf[8] >> 3);
    if !(1..=31).contains(&nbits) {
        return Err("Invalid live callback SWF frame rectangle".into());
    }
    let mut cursor = 8 + (5 + 4 * nbits).div_ceil(8) + 4;
    let mut tag_count = 0usize;
    let mut chunks = Vec::new();
    let insert_at = loop {
        if cursor + 2 > swf.len() {
            return Err("Root frame ShowFrame not found for live menu callbacks".into());
        }
        let start = cursor;
        let header = u16::from_le_bytes(swf[cursor..cursor + 2].try_into().unwrap());
        cursor += 2;
        let code = header >> 6;
        let mut length = usize::from(header & 63);
        if length == 63 {
            if cursor + 4 > swf.len() {
                return Err("Truncated first-frame SWF tag".into());
            }
            length = u32::from_le_bytes(swf[cursor..cursor + 4].try_into().unwrap()) as usize;
            cursor += 4;
        }
        if length > swf.len() - cursor {
            return Err("Invalid first-frame SWF tag size".into());
        }
        if code == 1 {
            break start;
        }
        if code == 0 || tag_count >= 80_000 {
            return Err("Live callback insertion needs an original first frame".into());
        }
        if code == 12 {
            chunks.push(&swf[cursor..cursor + length]);
        }
        cursor += length;
        tag_count += 1;
    };
    // The whitelist is fixed and each callback is registered only if the
    // original first frame declares the expected exact arity in AVM1 bytecode.
    const PRESETS: &[PreviewPreset] = &[
        PreviewPreset::OpenScreen,
        PreviewPreset::ShowEngagement,
        PreviewPreset::HideEngagement,
        PreviewPreset::ShowSigningIn,
        PreviewPreset::HideSigningIn,
        PreviewPreset::Back,
        PreviewPreset::ReturnToMainMenu,
        PreviewPreset::ShowPatchNotes,
        PreviewPreset::HidePatchNotes,
        PreviewPreset::ShowNews,
        PreviewPreset::HideNews,
        PreviewPreset::ShowChallenges,
        PreviewPreset::HideChallenges,
    ];
    let mut allowed = Vec::new();
    let mut emitted = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for preset in PRESETS {
        let Some((function, argument)) = preset.script_call() else {
            continue;
        };
        if !seen.insert(function) {
            // Both show/hide toggle presets share the same callback.
            if allowed.iter().any(|item: &PreviewPreset| {
                item.script_call().is_some_and(|(name, _)| name == function)
            }) {
                allowed.push(*preset);
            }
            continue;
        }
        let expected = usize::from(argument.is_some());
        let found = chunks
            .iter()
            .filter_map(|actions| defined_function_arity(actions, function))
            .collect::<Vec<_>>();
        let populated_menu = function == "openScreen"
            && !found.is_empty()
            && found.iter().all(|&count| count == 7)
            && [
                ("clearMenuState", 1),
                ("pushMenuState", 0),
                ("transitionToMainMenu", 0),
            ]
            .iter()
            .all(|(name, arity)| {
                let matches = chunks
                    .iter()
                    .filter_map(|a| defined_function_arity(a, name))
                    .collect::<Vec<_>>();
                !matches.is_empty() && matches.iter().all(|n| n == arity)
            });
        if !populated_menu && (found.is_empty() || found.iter().any(|&count| count != expected)) {
            continue;
        }
        let method = format!("evolvePreview_{}", function);
        let wrapper = format!("evolveCallback_{}", function);
        // DefineFunction(wrapper, optional boolean parameter) body.
        let mut body = Vec::new();
        if populated_menu {
            // Source MainMenu.xml + main_menu.gfx define this seven-argument
            // lifecycle. Values describe an offline preview, not game state.
            avm1_push_boolean(&mut body, false);
            avm1_push_i32(&mut body, 1);
            avm1_push_string(&mut body, "clearMenuState")?;
            body.extend_from_slice(&[0x3d, 0x17]);
            avm1_push_string(&mut body, "menuButtonData")?;
            avm1_push_string(&mut body, "menuData")?;
            body.push(0x1c);
            body.push(0x1d); // SetVariable: original pending menu array.
            // The original layout compares these labels to choose featured
            // button dimensions. Translate both sides of that comparison.
            for (target, key) in [
                ("MODE_NAME_PLAY", "play"),
                ("MODE_NAME_TRAINING", "training"),
            ] {
                avm1_push_string(&mut body, target)?;
                avm1_push_string(&mut body, "menuLabels")?;
                body.push(0x1c);
                avm1_push_string(&mut body, key)?;
                body.extend_from_slice(&[0x4e, 0x1d]);
            }
            avm1_push_boolean(&mut body, false); // showNews
            avm1_push_string(&mut body, "")?; // background_video: separately staged
            avm1_push_boolean(&mut body, true); // cinematic already playing
            for _ in 0..4 {
                avm1_push_boolean(&mut body, false);
            }
            avm1_push_i32(&mut body, 7);
            avm1_push_string(&mut body, "openScreen")?;
            body.extend_from_slice(&[0x3d, 0x17]);
            // openScreen populates the original source-defined menu before
            // pushMenuState displays it. transitionToMainMenu is a separate
            // original navigation operation; calling it immediately after
            // opening the populated root can clear/hide those fresh controls.
            avm1_push_i32(&mut body, 0);
            avm1_push_string(&mut body, "pushMenuState")?;
            body.extend_from_slice(&[0x3d, 0x17]);
        } else {
            if argument.is_some() {
                avm1_push_string(&mut body, "value")?;
                body.push(0x1c);
            }
            avm1_push_i32(&mut body, expected as i32);
            avm1_push_string(&mut body, function)?;
            body.extend_from_slice(&[0x3d, 0x17]);
        }
        avm1_push_boolean(&mut body, true);
        body.extend_from_slice(&[0x3e, 0]); // Return(true), End
        let mut def = Vec::new();
        def.extend_from_slice(wrapper.as_bytes());
        def.push(0);
        def.extend_from_slice(&((if populated_menu { 2 } else { expected }) as u16).to_le_bytes());
        if populated_menu {
            def.extend_from_slice(b"menuData\0menuLabels\0");
        } else if expected == 1 {
            def.extend_from_slice(b"value\0");
        }
        def.extend_from_slice(&(body.len() as u16).to_le_bytes());
        emitted.push(0x9b);
        emitted.extend_from_slice(&(def.len() as u16).to_le_bytes());
        emitted.extend_from_slice(&def);
        emitted.extend_from_slice(&body);

        // flash.external.ExternalInterface.addCallback(
        //     "evolvePreview_function", _root, wrapper)
        avm1_push_string(&mut emitted, &wrapper)?;
        emitted.push(0x1c); // GetVariable -> wrapper Function
        avm1_push_string(&mut emitted, "_root")?;
        emitted.push(0x1c); // GetVariable -> receiver
        avm1_push_string(&mut emitted, &method)?;
        avm1_push_i32(&mut emitted, 3);
        avm1_push_string(&mut emitted, "flash")?;
        emitted.push(0x1c); // GetVariable
        avm1_push_string(&mut emitted, "external")?;
        emitted.push(0x4e); // GetMember
        avm1_push_string(&mut emitted, "ExternalInterface")?;
        emitted.push(0x4e); // GetMember
        avm1_push_string(&mut emitted, "addCallback")?;
        emitted.extend_from_slice(&[0x52, 0x17]); // CallMethod; Pop
        // UI-local submenu navigation retains the original movie's
        // pushMenuState implementation. It is exposed only after exact
        // first-frame verification of the seven-argument main-menu lifecycle.
        // No native ExternalInterface game command is bound or executed.
        if populated_menu {
            let mut submenu_body = Vec::new();
            avm1_push_string(&mut submenu_body, "menuButtonData")?;
            avm1_push_string(&mut submenu_body, "menuData")?;
            submenu_body.push(0x1c); // GetVariable: supplied source rows
            submenu_body.push(0x1d); // SetVariable: original pending rows
            avm1_push_i32(&mut submenu_body, 0);
            avm1_push_string(&mut submenu_body, "pushMenuState")?;
            submenu_body.extend_from_slice(&[0x3d, 0x17]); // CallFunction, Pop
            avm1_push_boolean(&mut submenu_body, true);
            submenu_body.extend_from_slice(&[0x3e, 0]); // Return, End

            let mut submenu_def = b"evolveCallback_pushSourceMenu\0".to_vec();
            submenu_def.extend_from_slice(&1u16.to_le_bytes());
            submenu_def.extend_from_slice(b"menuData\0");
            submenu_def.extend_from_slice(&(submenu_body.len() as u16).to_le_bytes());
            emitted.push(0x9b); // DefineFunction
            emitted.extend_from_slice(&(submenu_def.len() as u16).to_le_bytes());
            emitted.extend_from_slice(&submenu_def);
            emitted.extend_from_slice(&submenu_body);

            avm1_push_string(&mut emitted, "evolveCallback_pushSourceMenu")?;
            emitted.push(0x1c); // GetVariable: wrapper
            avm1_push_string(&mut emitted, "_root")?;
            emitted.push(0x1c);
            avm1_push_string(&mut emitted, "evolvePreview_pushSourceMenu")?;
            avm1_push_i32(&mut emitted, 3);
            avm1_push_string(&mut emitted, "flash")?;
            emitted.push(0x1c);
            avm1_push_string(&mut emitted, "external")?;
            emitted.push(0x4e);
            avm1_push_string(&mut emitted, "ExternalInterface")?;
            emitted.push(0x4e);
            avm1_push_string(&mut emitted, "addCallback")?;
            emitted.extend_from_slice(&[0x52, 0x17]); // CallMethod, Pop
        }
        allowed.push(*preset);
    }
    // Read-only ActionScript probes for a missing Training image-card menu.
    // Verify the authored zero-argument source function before registering
    // a callback. Merely changing URL suffixes cannot cause a loader call
    // when shouldUseImageButtons() returned false.
    let image_branch_arity = chunks
        .iter()
        .filter_map(|actions| defined_function_arity(actions, "shouldUseImageButtons"))
        .collect::<Vec<_>>();
    if !image_branch_arity.is_empty() && image_branch_arity.iter().all(|&arity| arity == 0) {
        let mut body = Vec::new();
        avm1_push_i32(&mut body, 0);
        avm1_push_string(&mut body, "shouldUseImageButtons")?;
        body.push(0x3d); // Original CallFunction; leave the boolean result.
        body.extend_from_slice(&[0x3e, 0]); // Return, End.
        let mut definition = b"evolveCallback_probeImageButtons\0".to_vec();
        definition.extend_from_slice(&0u16.to_le_bytes());
        definition.extend_from_slice(&(body.len() as u16).to_le_bytes());
        emitted.push(0x9b);
        emitted.extend_from_slice(&(definition.len() as u16).to_le_bytes());
        emitted.extend_from_slice(&definition);
        emitted.extend_from_slice(&body);
        avm1_push_string(&mut emitted, "evolveCallback_probeImageButtons")?;
        emitted.push(0x1c);
        avm1_push_string(&mut emitted, "_root")?;
        emitted.push(0x1c);
        avm1_push_string(&mut emitted, "evolvePreview_probeImageButtons")?;
        avm1_push_i32(&mut emitted, 3);
        avm1_push_string(&mut emitted, "flash")?;
        emitted.push(0x1c);
        avm1_push_string(&mut emitted, "external")?;
        emitted.push(0x4e);
        avm1_push_string(&mut emitted, "ExternalInterface")?;
        emitted.push(0x4e);
        avm1_push_string(&mut emitted, "addCallback")?;
        emitted.extend_from_slice(&[0x52, 0x17]);
    }
    // menuButtonData is the original root variable established by the
    // verified seven-argument openScreen lifecycle. Inspect its length,
    // without modifying menu state, inventing cards or executing game code.
    if allowed.contains(&PreviewPreset::OpenScreen) {
        let mut body = Vec::new();
        avm1_push_string(&mut body, "menuButtonData")?;
        body.push(0x1c);
        avm1_push_string(&mut body, "length")?;
        body.push(0x4e);
        body.extend_from_slice(&[0x3e, 0]);
        let mut definition = b"evolveCallback_probeMenuRows\0".to_vec();
        definition.extend_from_slice(&0u16.to_le_bytes());
        definition.extend_from_slice(&(body.len() as u16).to_le_bytes());
        emitted.push(0x9b);
        emitted.extend_from_slice(&(definition.len() as u16).to_le_bytes());
        emitted.extend_from_slice(&definition);
        emitted.extend_from_slice(&body);
        avm1_push_string(&mut emitted, "evolveCallback_probeMenuRows")?;
        emitted.push(0x1c);
        avm1_push_string(&mut emitted, "_root")?;
        emitted.push(0x1c);
        avm1_push_string(&mut emitted, "evolvePreview_probeMenuRows")?;
        avm1_push_i32(&mut emitted, 3);
        avm1_push_string(&mut emitted, "flash")?;
        emitted.push(0x1c);
        avm1_push_string(&mut emitted, "external")?;
        emitted.push(0x4e);
        avm1_push_string(&mut emitted, "ExternalInterface")?;
        emitted.push(0x4e);
        avm1_push_string(&mut emitted, "addCallback")?;
        emitted.extend_from_slice(&[0x52, 0x17]);
    }
    // Preview-only interception of the ORIGINAL updateVideo(name) call.
    // The game uses a CRI-USM-backed Scaleform NetStream which Ruffle cannot
    // decode. Preserve the source function, emit its requested video name
    // through ActionTrace, and then call the original unmodified handler.
    // This is installed after the source movie defines updateVideo, and only
    // when its exact first-frame argument count has been verified.
    let video_arity = chunks
        .iter()
        .filter_map(|actions| defined_function_arity(actions, "updateVideo"))
        .collect::<Vec<_>>();
    if !video_arity.is_empty() && video_arity.iter().all(|&count| count == 1) {
        avm1_push_string(&mut emitted, "evolvePreview_original_updateVideo")?;
        avm1_push_string(&mut emitted, "updateVideo")?;
        emitted.push(0x1c); // GetVariable: original Function
        emitted.push(0x1d); // SetVariable: save function before defining wrapper

        let mut video_body = Vec::new();
        avm1_push_string(&mut video_body, "evolve-video:")?;
        avm1_push_string(&mut video_body, "name")?;
        video_body.push(0x1c); // GetVariable: original requested name
        video_body.push(0x47); // ActionAdd2: prefix + name
        video_body.push(0x26); // ActionTrace: observed on the page, no native dispatch

        avm1_push_string(&mut video_body, "name")?;
        video_body.push(0x1c);
        avm1_push_i32(&mut video_body, 1);
        avm1_push_string(&mut video_body, "evolvePreview_original_updateVideo")?;
        video_body.push(0x3d); // CallFunction: unchanged original implementation
        video_body.extend_from_slice(&[0x3e, 0]); // Return source function result, End

        let mut definition = b"updateVideo\0".to_vec();
        definition.extend_from_slice(&1u16.to_le_bytes());
        definition.extend_from_slice(b"name\0");
        definition.extend_from_slice(&(video_body.len() as u16).to_le_bytes());
        emitted.push(0x9b); // DefineFunction
        emitted.extend_from_slice(&(definition.len() as u16).to_le_bytes());
        emitted.extend_from_slice(&definition);
        emitted.extend_from_slice(&video_body);
    }
    if emitted.is_empty() {
        return Ok((swf.to_vec(), allowed));
    }
    emitted.push(0); // End DoAction block.
    let mut tag = ((12u16 << 6) | 63).to_le_bytes().to_vec();
    tag.extend_from_slice(&(emitted.len() as u32).to_le_bytes());
    tag.extend_from_slice(&emitted);
    if swf.len().saturating_add(tag.len()) > MAX_DECOMPRESSED {
        return Err("Live menu callback injection exceeds SWF size limit".into());
    }
    let mut output = Vec::with_capacity(swf.len() + tag.len());
    output.extend_from_slice(&swf[..insert_at]);
    output.extend_from_slice(&tag);
    output.extend_from_slice(&swf[insert_at..]);
    let full_length = output.len() as u32;
    output[4..8].copy_from_slice(&full_length.to_le_bytes());
    Ok((output, allowed))
}

fn avm1_push_string(actions: &mut Vec<u8>, value: &str) -> Result<(), String> {
    if value.len() > 250 || value.contains('\0') {
        return Err("Unsafe AVM1 preview callback name".into());
    }
    let mut bytes = vec![0u8];
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
    actions.push(0x96);
    actions.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    actions.extend_from_slice(&bytes);
    Ok(())
}
fn avm1_push_i32(actions: &mut Vec<u8>, value: i32) {
    actions.extend_from_slice(&[0x96, 5, 0, 7]);
    actions.extend_from_slice(&value.to_le_bytes());
}
fn avm1_push_boolean(actions: &mut Vec<u8>, value: bool) {
    actions.extend_from_slice(&[0x96, 2, 0, 5, u8::from(value)]);
}

/// Insert one validated, UI-local original ActionScript call into a
/// temporary uncompressed Ruffle SWF. Never touch source GFX or call game
/// services. A function must be declared at the root before ShowFrame.
// Legacy one-shot action injector retained for regression tests only.
#[allow(dead_code)]
fn inject_preview_call(
    swf: &[u8],
    virtual_movie: &str,
    preset: PreviewPreset,
) -> Result<Vec<u8>, String> {
    let Some((function, argument)) = preset.script_call() else {
        return Ok(swf.to_vec());
    };
    if !virtual_movie.eq_ignore_ascii_case("Libs/UI/main_menu.gfx") {
        return Err("Preview functions are only defined for the original main_menu.gfx".into());
    }
    if swf.len() < 24 || swf.len() > MAX_DECOMPRESSED || &swf[..3] != b"FWS" {
        return Err(
            "Menu action testing requires an uncompressed, bounded main-menu preview".into(),
        );
    }
    if usize::try_from(u32::from_le_bytes(swf[4..8].try_into().unwrap())).ok() != Some(swf.len()) {
        return Err("Incorrect preview SWF length".into());
    }
    let nbits = usize::from(swf[8] >> 3);
    if !(1..=31).contains(&nbits) {
        return Err("Bad frame bounds in the main-menu movie".into());
    }
    let mut cursor = 8 + (5 + 4 * nbits).div_ceil(8) + 4;
    let mut arity = None;
    let mut insert_at = None;
    let mut tags = 0usize;
    while cursor + 2 <= swf.len() {
        let tag_start = cursor;
        let header = u16::from_le_bytes(swf[cursor..cursor + 2].try_into().unwrap());
        cursor += 2;
        let code = header >> 6;
        let mut size = usize::from(header & 63);
        if size == 63 {
            if cursor + 4 > swf.len() {
                return Err("Truncated main-menu long tag".into());
            }
            size = u32::from_le_bytes(swf[cursor..cursor + 4].try_into().unwrap()) as usize;
            cursor += 4;
        }
        if size > swf.len() - cursor {
            return Err("Truncated main-menu tag data".into());
        }
        if code == 12
            && let Some(n) = defined_function_arity(&swf[cursor..cursor + size], function)
            && arity.replace(n).is_some_and(|old| old != n)
        {
            return Err("Conflicting main-menu function declarations".into());
        }
        if code == 1 {
            insert_at = Some(tag_start);
            break;
        }
        tags += 1;
        if tags > 80_000 || code == 0 {
            break;
        }
        cursor += size;
    }
    let expected = usize::from(argument.is_some());
    if arity != Some(expected) {
        return Err(format!(
            "Original ActionScript function {function} with {expected} argument(s) was not verified"
        ));
    }
    let at = insert_at.ok_or("The original main-menu first frame was not found")?;
    // AVM1 CallFunction pops the function name, then argument count, then
    // arguments. Values are pushed in reverse order: arguments, count, name.
    let mut args = Vec::new();
    if let Some(argument) = argument {
        args.extend_from_slice(&[5, u8::from(argument)]);
    }
    args.push(7);
    args.extend_from_slice(&(expected as i32).to_le_bytes());
    args.push(0);
    args.extend_from_slice(function.as_bytes());
    args.push(0);
    let mut actions = vec![0x96];
    actions.extend_from_slice(&(args.len() as u16).to_le_bytes());
    actions.extend_from_slice(&args);
    actions.extend_from_slice(&[0x3d, 0x17, 0]);
    let mut tag = ((12u16 << 6) | 63).to_le_bytes().to_vec();
    tag.extend_from_slice(&(actions.len() as u32).to_le_bytes());
    tag.extend_from_slice(&actions);
    let new_len = swf.len() + tag.len();
    if new_len > MAX_DECOMPRESSED {
        return Err("Menu action preview exceeds the SWF size limit".into());
    }
    let mut output = Vec::with_capacity(new_len);
    output.extend_from_slice(&swf[..at]);
    output.extend_from_slice(&tag);
    output.extend_from_slice(&swf[at..]);
    output[4..8].copy_from_slice(&(new_len as u32).to_le_bytes());
    Ok(output)
}

#[derive(Debug)]
struct ProcessOwner {
    child: Mutex<Child>,
    _scratch: tempfile::TempDir,
}
impl Drop for ProcessOwner {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
#[derive(Debug, Clone)]
pub struct RunningPreview {
    process: Option<Arc<ProcessOwner>>,
    live: Option<Arc<gfx_live_web::LiveServer>>,
    pub restored_images: usize,
    pub skipped_images: usize,
    pub missing_imports: usize,
    // Historical one-shot preset for the standalone desktop fallback.
    #[allow(dead_code)]
    pub preset: PreviewPreset,
}
impl RunningPreview {
    pub fn is_running(&self) -> bool {
        if let Some(server) = &self.live {
            return server.is_running();
        }
        self.process.as_ref().is_some_and(|process| {
            process
                .child
                .lock()
                .is_ok_and(|mut child| child.try_wait().ok().flatten().is_none())
        })
    }

    pub fn reopen_live(&self) -> Result<(), String> {
        self.live
            .as_ref()
            .ok_or("Live preview is not active")?
            .reopen()
    }

    pub fn can_dispatch_live(&self, preset: PreviewPreset) -> bool {
        self.live
            .as_ref()
            .is_some_and(|server| server.supports(preset))
    }

    pub fn dispatch_live(&self, preset: PreviewPreset) -> Result<(), String> {
        self.live
            .as_ref()
            .ok_or("Live browser preview is not running")?
            .dispatch(preset)
    }
}
// Desktop Ruffle path validation is retained for fallback compatibility.
#[allow(dead_code)]
fn executable(path: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(path.trim());
    if !candidate.is_absolute()
        || !candidate.file_name().is_some_and(|n| {
            matches!(
                n.to_string_lossy().to_ascii_lowercase().as_str(),
                "ruffle.exe" | "ruffle" | "ruffle_desktop.exe" | "ruffle_desktop"
            )
        })
    {
        return Err(
            "Choose the absolute path to an installed Ruffle desktop executable (ruffle.exe)."
                .into(),
        );
    }
    let meta = fs::symlink_metadata(candidate).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() == 0 || meta.len() > MAX_PLAYER_BYTES {
        return Err("Ruffle must be a regular executable file (not a link).".into());
    }
    fs::canonicalize(candidate).map_err(|e| e.to_string())
}
fn safe_import(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 1024
        && !name.contains(['\\', ':'])
        && !name.starts_with('/')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && (name.to_ascii_lowercase().ends_with(".swf")
            || name.to_ascii_lowercase().ends_with(".gfx"))
}
/// Reheader to the SWF variant of the same compiled movie, without modifying
/// bytecode, frame instructions or authored display-list transformations.
fn flash_movie(movie: &[u8]) -> Result<Vec<u8>, String> {
    if movie.len() < 16 || movie.len() > MAX_DECOMPRESSED {
        return Err("Imported Scaleform movie exceeds the native source size limit".into());
    }
    let mut inspection = movie.to_vec();
    inspection[0] = match &movie[..3] {
        b"GFX" | b"FWS" => b'G',
        b"CFX" | b"CWS" => b'C',
        _ => return Err("Unsupported imported movie signature".into()),
    };
    inspection[1..3].copy_from_slice(b"FX");
    if inspection.len() <= MAX_SOURCE {
        inspect_gfx(&inspection).map_err(|e| e.to_string())?;
    } else {
        // Reconstructed preview-only bitmap characters can expand a checked
        // original beyond the game's 12 MiB source bound (up to 32 MiB).
        // Reject non-raw or inconsistent synthetic headers.
        let size = u32::from_le_bytes(inspection[4..8].try_into().unwrap()) as usize;
        if inspection[0] != b'G' || size != inspection.len() {
            return Err("Invalid oversized render-only SWF movie".into());
        }
    }
    let mut swf = movie.to_vec();
    swf[0] = if inspection[0] == b'G' { b'F' } else { b'C' };
    swf[1..3].copy_from_slice(b"WS");
    Ok(swf)
}
fn renderable_movie(
    bytes: &[u8],
    virtual_movie: &str,
    source: &Path,
    session: &Session,
) -> Result<(Vec<u8>, usize, usize, gfx_external_textures::SkipDetails), String> {
    // Use the exact same verified atlas reconstruction as the FFDec preview.
    // Ruffle can then rasterize preserved bitmap-fill shapes and run supported
    // AVM1/AVM2 actions in the original script tags.
    let mut canonical = bytes.to_vec();
    if canonical.len() < 3 {
        return Err("Movie signature is truncated".into());
    }
    match &canonical[..3] {
        b"FWS" => canonical[..3].copy_from_slice(b"GFX"),
        b"CWS" => canonical[..3].copy_from_slice(b"CFX"),
        _ => (),
    }
    let result = gfx_external_textures::prepare_preview(
        &canonical,
        virtual_movie,
        source,
        session.library_root.as_deref(),
        session.source_build.as_deref(),
    )?;
    Ok((
        flash_movie(&result.movie)?,
        result.converted,
        result.skipped,
        result.skip_details,
    ))
}
fn relative_import(parent: &str, child: &str) -> Result<String, String> {
    // A parent is a directory, not a .gfx/.swf file. Validate the path
    // segments by appending a synthetic file name for the shared validator.
    if (!parent.is_empty() && !safe_import(&format!("{parent}/source.gfx"))) || !safe_import(child)
    {
        return Err("Unsafe Scaleform import reference".into());
    }
    let relative = if parent.is_empty() {
        child.to_owned()
    } else {
        format!("{parent}/{child}")
    };
    if !safe_import(&relative) || relative.len() > 1024 {
        return Err("Nested Scaleform import exceeds safe path constraints".into());
    }
    Ok(relative)
}

fn stage_import(
    session: &Session,
    reference: &str,
    output: &Path,
) -> Result<Option<(usize, usize, gfx_external_textures::SkipDetails)>, String> {
    if !safe_import(reference) || reference.eq_ignore_ascii_case("movie.swf") {
        return Err("The movie contains an unsafe or colliding GFX/SWF import".into());
    }
    let source_root = fs::canonicalize(session.folder.path()).map_err(|e| e.to_string())?;
    let candidate = source_root.join(reference);
    let Ok(real) = fs::canonicalize(candidate) else {
        return Ok(None);
    };
    if !real.starts_with(&source_root) {
        return Err("Imported movie escaped the GFX scratch directory".into());
    }
    let metadata = fs::symlink_metadata(&real).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_SOURCE as u64 {
        return Err("Imported movie is not a regular bounded file".into());
    }
    let bytes = fs::read(&real).map_err(|e| e.to_string())?;
    let virtual_path = session
        .relative
        .rsplit_once('/')
        .map(|(parent, _)| format!("{parent}/{reference}"))
        .unwrap_or_else(|| reference.to_owned());
    let (swf, converted, skipped, details) =
        renderable_movie(&bytes, &virtual_path, &real, session)?;
    let target = output.join(reference);
    let parent = target
        .parent()
        .ok_or("Invalid imported movie destination")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    fs::write(target, swf).map_err(|e| e.to_string())?;
    Ok(Some((converted, skipped, details)))
}
/// Follow original movie imports recursively, including font/glyph movies
/// imported by imported font libraries. All bytes are resolved from the same
/// source build into scratch copies. Never guess a missing font or scan the
/// user's installed game.
fn stage_live_import_graph(
    session: &Session,
    output: &Path,
) -> Result<
    (
        usize,
        usize,
        usize,
        Vec<String>,
        gfx_external_textures::SkipDetails,
    ),
    String,
> {
    let mut pending = session
        .info
        .imports
        .iter()
        .cloned()
        .map(|name| (String::new(), name, 0usize))
        .collect::<VecDeque<_>>();
    let mut visited = HashSet::new();
    let mut imported = Vec::<String>::new();
    let (mut restored, mut skipped, mut missing) = (0usize, 0usize, 0usize);
    let mut skip_details = gfx_external_textures::SkipDetails::default();
    while let Some((parent, name, depth)) = pending.pop_front() {
        let relative = relative_import(&parent, &name)?;
        if !visited.insert(relative.to_ascii_lowercase()) {
            continue;
        }
        if visited.len() > MAX_IMPORTS {
            return Err("Original GFX import graph exceeds 128 linked movies".into());
        }
        imported.push(relative.clone());
        match stage_import(session, &relative, output) {
            Ok(Some((images, unresolved, details))) => {
                restored += images;
                skipped += unresolved;
                skip_details.accumulate(&details);
            }
            Ok(None) => {
                missing += 1;
                continue;
            }
            Err(error) => return Err(format!("Imported Scaleform movie {name}: {error}")),
        }
        if depth >= 3 {
            continue;
        }
        let source = session.folder.path().join(&relative);
        let Ok(mut original) = fs::read(&source) else {
            continue;
        };
        if original.len() < 16 || original.len() > MAX_SOURCE {
            continue;
        }
        match &original[..3] {
            b"FWS" => original[..3].copy_from_slice(b"GFX"),
            b"CWS" => original[..3].copy_from_slice(b"CFX"),
            b"GFX" | b"CFX" => (),
            _ => continue,
        }
        let Ok(details) = inspect_gfx(&original) else {
            continue;
        };
        if details.imports.is_empty() {
            continue;
        }
        // The parent movie itself was manifest/same-folder verified before
        // following any nested references, and all nested members go through
        // that same restricted lookup and path validation.
        let virtual_path = session
            .relative
            .rsplit_once('/')
            .map(|(base, _)| format!("{base}/{relative}"))
            .unwrap_or_else(|| relative.clone());
        let next_parent = relative
            .rsplit_once('/')
            .map(|(folder, _)| folder.to_owned())
            .unwrap_or_default();
        let source_parent = session.folder.path().join(&next_parent);
        crate::gfx_dependencies::stage_imports(
            &details.imports,
            &virtual_path,
            &source,
            &source_parent,
            session.library_root.as_deref(),
            session.source_build.as_deref(),
        )?;
        for child in details.imports {
            pending.push_back((next_parent.clone(), child, depth + 1));
        }
    }
    imported.sort();
    Ok((restored, skipped, missing, imported, skip_details))
}

/// Resolve only exact, same-build UI_Assets manifest entries. Original PAKs
/// can store GFX movies at their archive root even though the game's virtual
/// UI mount is Libs/UI. Do not search unrelated archives or guess by basename.
fn verified_ui_asset_movie(
    root: &Path,
    build: &str,
    filename: &str,
) -> Option<evolve_core::LibraryReference> {
    use evolve_core::LibraryReferenceStatus;
    use evolve_core::library_catalog::resolve_imported_reference_in_archive;
    if build.trim().is_empty()
        || filename.is_empty()
        || !filename.ends_with(".gfx")
        || !filename
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.')
    {
        return None;
    }
    let mut found = None;
    for candidate in [
        format!("Libs/UI/{filename}"),
        format!("files/Libs/UI/{filename}"),
        filename.to_owned(),
        format!("files/{filename}"),
    ] {
        match resolve_imported_reference_in_archive(root, &candidate, build, "UI_Assets.pak")
            .ok()?
        {
            LibraryReferenceStatus::Found(hit) => {
                if found.is_some() {
                    return None; // Two virtual members: refuse ambiguous source.
                }
                found = Some(hit);
            }
            LibraryReferenceStatus::Ambiguous(_) => return None,
            LibraryReferenceStatus::Missing => {}
        }
    }
    found
}

/// Stage other *source-verified* UI_Assets movies next to the original menu.
/// These are manually browsable preview screens, not invented mappings from
/// native MainMenuScreen.xml commands to a Flash movie or game service.
/// Only an exact, unambiguous, matching-build virtual file is accepted.
fn stage_linked_screens(session: &Session, scratch: &Path) -> Vec<serde_json::Value> {
    const CANDIDATES: &[(&str, &str, &str)] = &[
        ("options", "Options", "options.gfx"),
        ("settings", "Settings", "settings.gfx"),
        ("challenges", "Challenges", "ChallengesPopup.gfx"),
        (
            "custom-match",
            "Custom match options",
            "custom_match_options.gfx",
        ),
        ("credits", "Credits", "credits.gfx"),
    ];
    let (Some(root), Some(build)) = (
        session.library_root.as_deref(),
        session.source_build.as_deref(),
    ) else {
        return Vec::new();
    };
    let mut screens = Vec::new();
    for &(id, title, file) in CANDIDATES {
        let Some(found) = verified_ui_asset_movie(root, build, file) else {
            continue;
        };
        let virtual_path = found.relative_path.clone();
        // A linked screen gets its own scratch hierarchy so its original
        // relative ImportAssets paths cannot collide with main_menu.gfx.
        let destination = scratch.join("screens").join(id);
        let screen = match crate::gfx_editor::create(
            PathBuf::from(found.absolute_path),
            virtual_path.clone(),
            None,
            Some(root.to_path_buf()),
            Some(build.to_owned()),
        ) {
            Ok(screen) => screen,
            Err(_) => continue,
        };
        let Ok(bytes) = fs::read(&screen.editable) else {
            continue;
        };
        let Ok((movie, _restored, _skipped, _bitmap_details)) =
            renderable_movie(&bytes, &screen.relative, &screen.source, &screen)
        else {
            continue;
        };
        if fs::create_dir_all(&destination).is_err() {
            continue;
        }
        if fs::write(destination.join("movie.swf"), movie).is_err() {
            continue;
        }
        // Imported font/movies and their original DDS tiles are reconstructed
        // using the exact same bounded pipeline as the root preview.
        let mut fonts = Vec::<serde_json::Value>::new();
        if let Ok((_, _, _, imports, _)) = stage_live_import_graph(&screen, &destination) {
            let _ = localize_imported_initial_text(&screen, &destination, &imports);
            for relative in imports
                .iter()
                .filter(|p| p.to_ascii_lowercase().contains("font"))
            {
                let font = destination.join(relative);
                if let Some(count) = original_font_definitions(&font)
                    && count > 0
                {
                    fonts.push(serde_json::json!({
                        "movie": format!("screens/{id}/{relative}"),
                        "staged": true,
                        "define_font_tags": count
                    }));
                }
            }
        }
        let ui_element =
            gfx_external_textures::associated_ui_data(&virtual_path, Some(root), Some(build));
        screens.push(serde_json::json!({
            "id": id,
            "title": title,
            "movie": format!("screens/{id}/movie.swf"),
            "fonts": fonts,
            "virtual_source": virtual_path,
            "ui_element": ui_element,
            "verification": "Exact same-build Asset Library member; native screen routing not verified."
        }));
    }
    screens
}

#[derive(Default)]
struct ImportedTextCounts {
    movies: usize,
    localized: usize,
    unresolved: usize,
    fields: usize,
    skipped: usize,
}

/// The root menu's translation pass cannot see the initial strings in nested
/// source movies. Resolve these keys from the exact selected build and patch
/// only the imported, temporary SWF copies; do not alter runtime ActionScript
/// assignments, source files, font libraries or game installation contents.
fn localize_imported_initial_text(
    session: &Session,
    root: &Path,
    imports: &[String],
) -> Result<ImportedTextCounts, String> {
    let mut result = ImportedTextCounts::default();
    if !session
        .relative
        .eq_ignore_ascii_case("Libs/UI/main_menu.gfx")
    {
        return Ok(result);
    }
    let mut keys = std::collections::BTreeSet::<String>::new();
    for name in imports {
        if !safe_import(name) {
            return Err("Unsafe imported movie localization path".into());
        }
        let source = root.join(name);
        if !source.is_file() {
            continue;
        }
        let bytes = fs::read(&source).map_err(|e| e.to_string())?;
        match crate::gfx_menu_text::authored_keys(&bytes) {
            Ok(found) => {
                keys.extend(found);
                if keys.len() > 1024 {
                    return Err("Imported menu translation requests exceed supported bounds".into());
                }
            }
            Err(_) => result.skipped += 1,
        }
    }
    if keys.is_empty() {
        return Ok(result);
    }
    let keys = keys.into_iter().collect::<Vec<_>>();
    let localization = crate::gfx_localization::menu_labels(
        session.library_root.as_deref(),
        session.source_build.as_deref(),
        &keys,
    );
    for name in imports {
        let target = root.join(name);
        if !target.is_file() {
            continue;
        }
        let bytes = fs::read(&target).map_err(|e| e.to_string())?;
        match crate::gfx_menu_text::replace_authored_text(&bytes, &localization) {
            Ok((patched, changes)) => {
                result.movies += 1;
                result.localized += changes.patched;
                result.unresolved += changes.unresolved;
                result.fields += changes.original_edit_fields;
                if changes.patched > 0 {
                    fs::write(target, patched).map_err(|e| e.to_string())?;
                }
            }
            Err(_) => result.skipped += 1,
        }
    }
    Ok(result)
}

/// Start a distinct Ruffle window on a temporary, validated copy of the GFX.
///
/// Host FSCommands and Evolve native Scaleform ExternalInterface are NOT
/// emulated; Ruffle's empty interface is used rather than routing any
/// original game action to an installed client or unrestricted filesystem.
/// The live process and preview directory are owned by RunningPreview.
// Legacy standalone Ruffle desktop runner: live controls use launch_live instead.
#[allow(dead_code)]
pub fn launch(
    session: &Session,
    ruffle_exe: &str,
    preset: PreviewPreset,
) -> Result<RunningPreview, String> {
    let binary = executable(ruffle_exe)?;
    let root = tempfile::Builder::new()
        .prefix("evolve-gfx-interactive-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let input = fs::read(&session.editable).map_err(|e| e.to_string())?;
    let (swf, mut converted, mut skipped, _) =
        renderable_movie(&input, &session.relative, &session.source, session)?;
    let root_movie = root.path().join("movie.swf");
    let movie = inject_preview_call(&swf, &session.relative, preset)?;
    fs::write(&root_movie, movie).map_err(|e| e.to_string())?;
    let mut missing = 0usize;
    for name in session.info.imports.iter().take(MAX_IMPORTS) {
        match stage_import(session, name, root.path()) {
            Ok(Some((images, skip, _))) => {
                converted += images;
                skipped += skip;
            }
            Ok(None) => missing += 1,
            Err(e) => return Err(format!("Imported Scaleform movie {name}: {e}")),
        }
    }
    // The movie executes *only* in this external test player. No game DLLs,
    // registered game paths, credentials, or network bridges are handed off.
    let child = Command::new(binary)
        .arg("--dummy-external-interface")
        .arg("--open-url-mode")
        .arg("deny")
        .arg("--filesystem-access-mode")
        .arg("ask")
        .arg(&root_movie)
        .current_dir(root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Unable to launch Ruffle desktop: {e}"))?;
    Ok(RunningPreview {
        process: Some(Arc::new(ProcessOwner {
            child: Mutex::new(child),
            _scratch: root,
        })),
        live: None,
        restored_images: converted,
        skipped_images: skipped,
        missing_imports: missing,
        preset,
    })
}

/// Read source-verified imported font movies for diagnostics, including
/// ordinary CWS compressed SWFs produced by the Stage 2 import graph.
/// The bounded decoded bytes are never written back to the original movie.
fn uncompressed_font_movie(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 16 || bytes.len() > MAX_DECOMPRESSED {
        return None;
    }
    if &bytes[..3] == b"FWS" {
        return (u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize == bytes.len())
            .then_some(bytes);
    }
    if &bytes[..3] != b"CWS" {
        return None;
    }
    let declared = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
    if !(16..=MAX_DECOMPRESSED).contains(&declared) {
        return None;
    }
    let mut output = Vec::with_capacity(declared);
    output.extend_from_slice(b"FWS");
    output.extend_from_slice(&bytes[3..8]);
    flate2::read::ZlibDecoder::new(&bytes[8..])
        .take(MAX_DECOMPRESSED as u64)
        .read_to_end(&mut output)
        .ok()?;
    (output.len() == declared).then_some(output)
}

/// Count original embedded SWF DefineFont definitions in the isolated font
/// import. An actual font definition is stronger than mere path resolution,
/// but still not proof that Ruffle reproduced Scaleform's font renderer.
fn original_font_definitions(path: &Path) -> Option<usize> {
    let bytes = uncompressed_font_movie(path)?;
    let nbits = usize::from(bytes[8] >> 3);
    if !(1..=31).contains(&nbits) {
        return None;
    }
    let mut cursor = 8 + (5 + 4 * nbits).div_ceil(8) + 4;
    let mut found = 0usize;
    let mut tags = 0usize;
    while cursor + 2 <= bytes.len() {
        let header = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]);
        cursor += 2;
        let kind = header >> 6;
        let mut length = usize::from(header & 63);
        if length == 63 {
            if cursor + 4 > bytes.len() {
                return None;
            }
            length = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().ok()?) as usize;
            cursor += 4;
        }
        if length > bytes.len() - cursor || tags >= 80_000 {
            return None;
        }
        if matches!(kind, 10 | 48 | 75 | 91) {
            found += 1;
        }
        cursor += length;
        tags += 1;
        if kind == 0 {
            return (cursor == bytes.len()).then_some(found);
        }
    }
    None
}

/// Return source-authored font identities from an imported SWF. Font IDs
/// are local to each movie: matching a name does not itself bind an imported
/// glyph definition to a root movie TextField or prove Ruffle rendered it.
fn original_font_inventory(path: &Path) -> Option<Vec<serde_json::Value>> {
    let bytes = uncompressed_font_movie(path)?;
    let nbits = usize::from(bytes[8] >> 3);
    if !(1..=31).contains(&nbits) {
        return None;
    }
    let mut cursor = 8 + (5 + 4 * nbits).div_ceil(8) + 4;
    let mut inventory = Vec::new();
    let mut tags = 0usize;
    while cursor + 2 <= bytes.len() && tags < 80_000 {
        let header = u16::from_le_bytes([bytes[cursor], bytes[cursor + 1]]);
        cursor += 2;
        let kind = header >> 6;
        let mut len = usize::from(header & 63);
        if len == 63 {
            if cursor + 4 > bytes.len() {
                return None;
            }
            len = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().ok()?) as usize;
            cursor += 4;
        }
        if len > bytes.len() - cursor {
            return None;
        }
        let tag = &bytes[cursor..cursor + len];
        if matches!(kind, 48 | 75) && tag.len() >= 7 {
            let id = u16::from_le_bytes([tag[0], tag[1]]);
            let flags = tag[2];
            let name_len = usize::from(tag[4]);
            if 5 + name_len + 2 > tag.len() || name_len > 128 {
                return None;
            }
            let name = std::str::from_utf8(&tag[5..5 + name_len]).ok()?;
            if name.chars().any(char::is_control) {
                return None;
            }
            let glyphs = u16::from_le_bytes([tag[5 + name_len], tag[6 + name_len]]);
            inventory.push(serde_json::json!({
                "font_id": id,
                "font_name": name,
                "glyphs": glyphs,
                "define_font_tag": kind,
                "bold": flags & 1 != 0,
                "italic": flags & 2 != 0
            }));
        } else if kind == 10 && tag.len() >= 2 {
            inventory.push(serde_json::json!({
                "font_id": u16::from_le_bytes([tag[0], tag[1]]),
                "font_name": null,
                "glyphs": null,
                "define_font_tag": kind
            }));
        } else if kind == 91 && tag.len() >= 4 {
            let id = u16::from_le_bytes([tag[0], tag[1]]);
            let name_end = tag[3..].iter().position(|b| *b == 0)?;
            let name = std::str::from_utf8(&tag[3..3 + name_end]).ok()?;
            if name.len() > 128 || name.chars().any(char::is_control) {
                return None;
            }
            inventory.push(serde_json::json!({
                "font_id": id,
                "font_name": name,
                "glyphs": null,
                "define_font_tag": kind
            }));
        }
        // DefineFontName (88) is the source PostScript/full-name alias for
        // a font ID within this movie, not a system-font substitution request.
        if kind == 88 && tag.len() >= 4 {
            let id = u16::from_le_bytes([tag[0], tag[1]]);
            let end = tag[2..].iter().position(|b| *b == 0)?;
            let alias = std::str::from_utf8(&tag[2..2 + end]).ok()?;
            if alias.len() > 128 || alias.chars().any(char::is_control) {
                return None;
            }
            for font in &mut inventory {
                if font["font_id"].as_u64() == Some(u64::from(id)) {
                    font["postscript_name"] = serde_json::json!(alias);
                }
            }
        }
        if inventory.len() > 128 {
            return None;
        }
        cursor += len;
        tags += 1;
        if kind == 0 {
            return (cursor == bytes.len()).then_some(inventory);
        }
    }
    None
}

/// Count SWF-authored display-list effect *flags*, including nested sprites.
/// This is an evidence inventory, not a substitute for the Scaleform
/// renderer: per-frame ActionScript color changes, CryEngine shader passes,
/// and Ruffle's actual pixel output require independent capture/verification.
fn source_render_effect_inventory(movie: &[u8]) -> serde_json::Value {
    #[derive(Default)]
    struct Counters {
        placements: usize,
        color_transforms: usize,
        filter_lists: usize,
        blend_modes: usize,
        sprites: usize,
        incomplete: bool,
    }
    fn walk(bytes: &[u8], mut at: usize, depth: usize, stats: &mut Counters) -> bool {
        if depth > 8 {
            stats.incomplete = true;
            return true;
        }
        let mut tags = 0usize;
        while at + 2 <= bytes.len() && tags < 80_000 {
            let header = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
            at += 2;
            let kind = header >> 6;
            let mut size = usize::from(header & 63);
            if size == 63 {
                if at + 4 > bytes.len() {
                    return false;
                }
                size = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
                at += 4;
            }
            if size > bytes.len() - at {
                return false;
            }
            let tag = &bytes[at..at + size];
            match kind {
                26 if !tag.is_empty() => {
                    stats.placements += 1;
                    if tag[0] & 0x08 != 0 {
                        stats.color_transforms += 1;
                    }
                }
                70 if tag.len() >= 2 => {
                    stats.placements += 1;
                    if tag[0] & 0x08 != 0 {
                        stats.color_transforms += 1;
                    }
                    if tag[1] & 0x01 != 0 {
                        stats.filter_lists += 1;
                    }
                    if tag[1] & 0x02 != 0 {
                        stats.blend_modes += 1;
                    }
                }
                39 if tag.len() >= 4 => {
                    stats.sprites += 1;
                    if !walk(&tag[4..], 0, depth + 1, stats) {
                        return false;
                    }
                }
                _ => {}
            }
            at += size;
            tags += 1;
            if kind == 0 {
                return at == bytes.len();
            }
        }
        false
    }
    if movie.len() < 20 || movie.len() > MAX_DECOMPRESSED || &movie[..3] != b"FWS" {
        return serde_json::json!({"status": "Source SWF unavailable or not uncompressed"});
    }
    let nbits = usize::from(movie[8] >> 3);
    if !(1..=31).contains(&nbits) {
        return serde_json::json!({"status": "Invalid original SWF frame bounds"});
    }
    let start = 8 + (5 + 4 * nbits).div_ceil(8) + 4;
    if start > movie.len() {
        return serde_json::json!({"status": "Truncated original SWF header"});
    }
    let mut counters = Counters::default();
    let valid = walk(movie, start, 0, &mut counters);
    serde_json::json!({
        "status": if valid { "SWF flags inventoried" } else { "Malformed SWF tags" },
        "partial": counters.incomplete || !valid,
        "placement_count": counters.placements,
        "authored_color_transforms": counters.color_transforms,
        "authored_filter_lists": counters.filter_lists,
        "authored_blend_modes": counters.blend_modes,
        "nested_sprites": counters.sprites,
        "runtime_verified": false,
        "note": "Static authored tags only. Original dynamic color multipliers, noise, post effects, and Ruffle pixels remain unverified."
    })
}

/// Serialize *source-defined* UI_Data commands for an optional live browser
/// inspector. These do not become fabricated SWF display-list nodes. Commands
/// that require CryEngine remain read-only; only known child submenu links can
/// be traversed locally. This is diagnostics, NOT native menu emulation.
struct LiveMenuCounts {
    restored: usize,
    skipped: usize,
    missing: usize,
    traced_buttons: usize,
    bitmap_skips: gfx_external_textures::SkipDetails,
}
struct LiveMenuLabels<'a> {
    source: &'a serde_json::Value,
    text: &'a crate::gfx_menu_text::TextPatch,
    imported_text: &'a ImportedTextCounts,
    render_effects: &'a serde_json::Value,
    linked_screens: &'a [serde_json::Value],
    mode_images: &'a serde_json::Value,
}

fn write_live_menu_report(
    session: &Session,
    scratch: &Path,
    counts: LiveMenuCounts,
    verified_actions: &[PreviewPreset],
    imported_movies: &[String],
    background_video: serde_json::Value,
    labels: LiveMenuLabels<'_>,
) -> Result<(), String> {
    let entries = session
        .menu_commands
        .iter()
        .take(128)
        .map(|item| {
            serde_json::json!({
                "submenu": item.submenu,
                "label": item.label,
                "command": item.command,
                "child_submenu": item.child_submenu,
                "disabled_when": item.disabled_when,
                "hidden_when": item.hidden_when,
                "platform": item.platform
            })
        })
        .collect::<Vec<_>>();
    let unresolved = session
        .dependencies
        .iter()
        .filter(|item| {
            item.outcome.contains("Missing")
                || item.outcome.contains("unavailable")
                || item.outcome.contains("Ambiguous")
        })
        .take(128)
        .map(|item| item.reference.as_str())
        .collect::<Vec<_>>();
    // Imported font movies remain original same-build assets; resolving an
    // import does not prove Ruffle supports its Scaleform glyph tags.
    let font_imports = imported_movies
        .iter()
        .filter(|name| name.to_ascii_lowercase().contains("font"))
        .map(|name| {
            let found = session
                .dependencies
                .iter()
                .find(|entry| entry.reference.eq_ignore_ascii_case(name));
            let font_path = scratch.join(name);
            let inventory = original_font_inventory(&font_path);
            serde_json::json!({
                "movie": name,
                "staged": font_path.is_file(),
                "define_font_tags": original_font_definitions(&font_path),
                "font_definitions": inventory,
                "source": found.map_or("Source-defined nested import", |dependency| dependency.outcome.as_str())
            })
        })
        .collect::<Vec<_>>();
    let initialized_in_swf = verified_actions.contains(&PreviewPreset::ReturnToMainMenu);
    let original_open_screen = verified_actions.contains(&PreviewPreset::OpenScreen);
    let original_hide_patch_notes = verified_actions.contains(&PreviewPreset::HidePatchNotes);
    let original_hide_signing_in = verified_actions.contains(&PreviewPreset::HideSigningIn);
    let report = serde_json::json!({
        "source": "verified same-build UI_Data/MainMenuScreen.xml",
        "localization": labels.source,
        "authored_flash_text": {
            "fields": labels.text.original_edit_fields,
            "localized": labels.text.patched,
            "unresolved": labels.text.unresolved,
            "scope": "Only original DefineEditText initial strings; dynamic ActionScript labels require native host semantics."
        },
        "fonts": font_imports,
        "render_effects": labels.render_effects,
        "imported_flash_text": {
            "movies": labels.imported_text.movies,
            "localized": labels.imported_text.localized,
            "unresolved": labels.imported_text.unresolved,
            "fields": labels.imported_text.fields,
            "unreadable_or_unsupported": labels.imported_text.skipped,
            "scope": "Only source-authored DefineEditText initial strings in imported SWF previews; dynamic assignments remain unresolved."
        },
        "background_video": background_video,
        "linked_screens": labels.linked_screens,
        "mode_images": labels.mode_images,
        "initialization": {
            "verified_transition_to_main_menu": initialized_in_swf,
            "verified_open_screen": original_open_screen,
            "verified_source_submenus": original_open_screen,
            "verified_go_back": verified_actions.contains(&PreviewPreset::Back),
            "verified_hide_patch_notes": original_hide_patch_notes,
            "verified_hide_signing_in": original_hide_signing_in,
            "native_cryengine_host_available": false,
            "status": "Only original verified ActionScript handlers may initialize local menu state. Native game state is unavailable."
        },
        "menu_status": session.menu_command_status,
        "commands": entries,
        "resources": {
            "restored_dds_tiles": counts.restored,
            "instrumented_original_button_conditions": counts.traced_buttons,
            "skipped_dds_tiles": counts.skipped,
            "bitmap_skip_details": counts.bitmap_skips,
            "missing_imported_movies": counts.missing,
            "unresolved_imports": unresolved,
            "has_source_build": session.source_build.is_some(),
            "source_build_label": session.source_build,
            "has_asset_library": session.library_root.is_some()
        },
        "limitation": "This is a source-defined command inspector. The original SWF is unchanged. Native CryEngine game state, dynamic Scaleform UI initialization, and in-game localization are not reproduced."
    });
    let bytes = serde_json::to_vec(&report).map_err(|e| e.to_string())?;
    if bytes.len() > 512 * 1024 {
        return Err("Original menu definition report exceeds 512 KiB".into());
    }
    fs::write(scratch.join("preview-menu.json"), bytes).map_err(|e| e.to_string())
}

/// A persistent browser-based Ruffle preview. All selected controls use
/// an allowlisted live ExternalInterface callback instead of patch/restart.
pub fn launch_live(
    session: &Session,
    web_runtime: &str,
    ffmpeg_executable: &str,
) -> Result<RunningPreview, String> {
    let scratch = tempfile::Builder::new()
        .prefix("evolve-gfx-live-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let bytes = fs::read(&session.editable).map_err(|e| e.to_string())?;
    let (swf, mut converted, mut skipped, mut bitmap_skips) =
        renderable_movie(&bytes, &session.relative, &session.source, session)?;
    let main_menu = session
        .relative
        .eq_ignore_ascii_case("Libs/UI/main_menu.gfx");
    let mut lookup_tokens = session
        .menu_commands
        .iter()
        .map(|row| row.label.clone())
        .collect::<Vec<_>>();
    if main_menu {
        lookup_tokens.extend(crate::gfx_menu_text::authored_keys(&swf)?);
    }
    let localization = crate::gfx_localization::menu_labels(
        session.library_root.as_deref(),
        session.source_build.as_deref(),
        &lookup_tokens,
    );
    let (swf, localized_text) = if main_menu {
        crate::gfx_menu_text::replace_authored_text(&swf, &localization)?
    } else {
        (swf, crate::gfx_menu_text::TextPatch::default())
    };
    // Trace authored SWF button handlers before registering the original
    // live callbacks. No synthesized UI or native handler mapping is added.
    let render_effects = source_render_effect_inventory(&swf);
    let (inspected, traced_buttons) = if session
        .relative
        .eq_ignore_ascii_case("Libs/UI/main_menu.gfx")
    {
        trace_original_button_actions(&swf)?
    } else {
        (swf, 0)
    };
    let (movie, actions) = install_live_callbacks(&inspected, &session.relative)?;
    fs::write(scratch.path().join("movie.swf"), &movie).map_err(|e| e.to_string())?;
    let (imported_images, import_skipped, missing, imported_movies, imported_skips) =
        stage_live_import_graph(session, scratch.path())?;
    bitmap_skips.accumulate(&imported_skips);
    let imported_text = localize_imported_initial_text(session, scratch.path(), &imported_movies)?;
    converted += imported_images;
    skipped += import_skipped;
    // The original Stage 2 background is CRI USM in videos.pak; it is not
    // supposed to be reconstructed as a static Flash fill.
    let mut background_video = if session
        .relative
        .eq_ignore_ascii_case("Libs/UI/main_menu.gfx")
    {
        crate::gfx_menu_video::stage_main_menu(
            session.library_root.as_deref(),
            session.source_build.as_deref(),
            scratch.path(),
            ffmpeg_executable,
        )
    } else {
        serde_json::json!({"playable": false, "status": "Not the Stage 2 main menu"})
    };
    // The default Goliath clip remains independent from original submenu
    // requests. Collect exact same-build FrontEndVideoList sources without
    // converting unrelated or missing videos at startup.
    let (submenu_manifest, submenu_videos) = if main_menu {
        crate::gfx_menu_video::submenu_catalog(
            session.library_root.as_deref(),
            session.source_build.as_deref(),
            ffmpeg_executable,
        )
    } else {
        (
            serde_json::json!({"available": 0}),
            crate::gfx_menu_video::SubmenuVideoCatalog::default(),
        )
    };
    background_video["submenus"] = submenu_manifest;
    let linked_screens = if main_menu {
        stage_linked_screens(session, scratch.path())
    } else {
        Vec::new()
    };
    // Mode-selection artwork is loaded dynamically by the original
    // ImageLoading.loadBitmap("img://...") ActionScript, not by atlas 1008/1009.
    // Stage verified source images as PNGs for the Ruffle URL rewrite rule.
    let mut mode_images = if main_menu {
        crate::gfx_mode_images::stage(
            session.library_root.as_deref(),
            session.source_build.as_deref(),
            scratch.path(),
            &movie,
        )
    } else {
        serde_json::json!({"requested": 0, "staged": 0})
    };
    if main_menu {
        let names = mode_images["staged_names"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|name| name.as_str().map(ToOwned::to_owned))
            .collect::<Vec<_>>();
        // Ruffle cannot perform the engine's native Scaleform img:// loader.
        // Add scratch-only DefineBitsLossless2 + ExportAssets records with
        // original linkage names, so standard BitmapData.loadBitmap can find
        // the verified same-build artwork without modifying game assets.
        let (linked_movie, linkages, skipped_linkages) =
            gfx_external_textures::embed_mode_bitmap_linkages(&movie, scratch.path(), &names)?;
        // The original ImageLoading class uses MovieClipLoader.loadClip with
        // img:// DDS URLs. BitmapData ExportAssets linkages cannot satisfy
        // that call. Rewrite its single source-owned URL prefix only in the
        // isolated scratch SWF and serve verified PNGs at that virtual path.
        let (routed_movie, loadclip_urls) = if names.is_empty() {
            (linked_movie, 0)
        } else {
            gfx_external_textures::route_mode_image_loadclip_urls(&linked_movie)?
        };
        // Real card clips are loaded with MovieClipLoader, not ExportAssets.
        // Convert only the verified card URL suffixes to a browser-decodable
        // PNG in this temporary SWF, preserving authored focus/hover logic.
        let (browser_movie, png_paths) =
            gfx_external_textures::route_mode_image_png_paths(&routed_movie, &names)?;
        fs::write(scratch.path().join("movie.swf"), &browser_movie).map_err(|e| e.to_string())?;
        mode_images["flash_bitmap_linkages"] = serde_json::json!(linkages);
        mode_images["flash_linkage_skips"] = serde_json::json!(skipped_linkages);
        mode_images["source_movieclip_url_rewrites"] = serde_json::json!(loadclip_urls);
        mode_images["source_movieclip_png_paths"] = serde_json::json!(png_paths);
    }
    write_live_menu_report(
        session,
        scratch.path(),
        LiveMenuCounts {
            restored: converted,
            skipped,
            missing,
            traced_buttons,
            bitmap_skips,
        },
        &actions,
        &imported_movies,
        background_video,
        LiveMenuLabels {
            linked_screens: &linked_screens,
            mode_images: &mode_images,
            source: &localization,
            text: &localized_text,
            imported_text: &imported_text,
            render_effects: &render_effects,
        },
    )?;
    let live = gfx_live_web::LiveServer::start(scratch, actions, web_runtime, submenu_videos)?;
    Ok(RunningPreview {
        process: None,
        live: Some(Arc::new(live)),
        restored_images: converted,
        skipped_images: skipped,
        missing_imports: missing,
        preset: PreviewPreset::AsAuthored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_original_screen_can_resolve_archive_root_gfx() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pak = root.join("UI_Assets.pak__11111111111111111111");
        fs::create_dir_all(pak.join("files")).unwrap();
        let movie = b"GFX synthetic source movie";
        fs::write(pak.join("files/settings.gfx"), movie).unwrap();
        fs::write(
            pak.join("asset-library.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "name": "UI_Assets.pak", "source": "/game/UI_Assets.pak",
                "entries": [{"path":"settings.gfx", "bytes":movie.len()}]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("build-collections.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":1,
                "builds":{"stage-two":{"label":"Modded Stage 2","members":{"11111111111111111111":{}}}}
            })).unwrap()
        ).unwrap();
        let found = verified_ui_asset_movie(root, "Modded Stage 2", "settings.gfx").unwrap();
        assert_eq!(found.relative_path, "settings.gfx");
        assert!(verified_ui_asset_movie(root, "2012", "settings.gfx").is_none());
        assert!(verified_ui_asset_movie(root, "Modded Stage 2", "../settings.gfx").is_none());
        assert!(verified_ui_asset_movie(root, "Modded Stage 2", "made_up.gfx").is_none());
    }

    fn menu_fixture() -> Vec<u8> {
        menu_fixture_arity(0)
    }
    fn menu_fixture_arity(open_arity: u16) -> Vec<u8> {
        // Synthetic AVM1 DefineFunction root script (no original Evolve assets).
        let mut movie = b"FWS\x0f".to_vec();
        movie.extend_from_slice(&[0; 4]);
        // Valid 1920×1080 SWF RECT, at 30fps. Previously the synthetic
        // fixture used zero-sized stage bounds and failed inspect_gfx.
        let mut bits = [0u8; 5 + 4 * 17];
        let mut put = |offset: usize, width: usize, value: u32| {
            for i in 0..width {
                bits[offset + i] = ((value >> (width - i - 1)) & 1) as u8;
            }
        };
        put(0, 5, 17);
        for (i, value) in [0u32, 38400, 0, 21600].iter().enumerate() {
            put(5 + i * 17, 17, *value);
        }
        let mut rect = vec![0u8; bits.len().div_ceil(8)];
        for (i, bit) in bits.iter().enumerate() {
            rect[i / 8] |= bit << (7 - i % 8);
        }
        movie.extend_from_slice(&rect);
        movie.extend_from_slice(&7680u16.to_le_bytes());
        movie.extend_from_slice(&1u16.to_le_bytes());
        let mut action = Vec::new();
        for (name, params) in [
            ("openScreen", open_arity),
            ("clearMenuState", 1),
            ("pushMenuState", 0),
            ("shouldUseImageButtons", 0),
            ("showEngagementPrompt", 1u16),
            ("transitionToMainMenu", 0),
            ("goBack", 0),
        ] {
            let mut payload = name.as_bytes().to_vec();
            payload.push(0);
            payload.extend_from_slice(&params.to_le_bytes());
            for _ in 0..params {
                payload.extend_from_slice(b"show\0");
            }
            payload.extend_from_slice(&0u16.to_le_bytes()); // function body length
            action.push(0x9b);
            action.extend_from_slice(&(payload.len() as u16).to_le_bytes());
            action.extend_from_slice(&payload);
        }
        action.push(0);
        movie.extend_from_slice(&((12u16 << 6) | 63).to_le_bytes());
        movie.extend_from_slice(&(action.len() as u32).to_le_bytes());
        movie.extend_from_slice(&action);
        movie.extend_from_slice(&(1u16 << 6).to_le_bytes());
        movie.extend_from_slice(&0u16.to_le_bytes());
        let n = movie.len() as u32;
        movie[4..8].copy_from_slice(&n.to_le_bytes());
        movie
    }

    #[test]
    fn ui_data_browser_report_preserves_original_labels_and_never_fabricates_handlers() {
        let folder = Arc::new(tempfile::tempdir().unwrap());
        let mut movie = menu_fixture();
        movie[..3].copy_from_slice(b"GFX");
        let session = Session {
            source: folder.path().join("main_menu.gfx"),
            relative: "Libs/UI/main_menu.gfx".into(),
            editable: folder.path().join("movie.gfx"),
            folder: Arc::clone(&folder),
            project: None,
            original_sha: String::new(),
            info: inspect_gfx(&movie).unwrap(),
            layers: Vec::new(),
            dependencies: vec![crate::gfx_dependencies::Dependency {
                reference: "gfxfontlib_locfont.gfx".into(),
                outcome: "Missing verified source".into(),
            }],
            menu_commands: vec![crate::gfx_menu_map::MenuCommand {
                submenu: "Main".into(),
                label: "@ui_STARTGAME".into(),
                command: "Game_Start".into(),
                child_submenu: "Extras".into(),
                disabled_when: String::new(),
                hidden_when: String::new(),
                platform: String::new(),
            }],
            menu_command_status: "1 verified original menu entry".into(),
            library_root: None,
            source_build: None,
            edits: 0,
            applied: false,
        };
        write_live_menu_report(
            &session,
            folder.path(),
            LiveMenuCounts {
                restored: 5,
                skipped: 3,
                missing: 1,
                traced_buttons: 2,
                bitmap_skips: gfx_external_textures::SkipDetails::default(),
            },
            &[PreviewPreset::Back, PreviewPreset::HideSigningIn],
            &["gfxfontlib_locfont.gfx".into()],
            serde_json::json!({"playable": false, "status": "missing videos.pak"}),
            LiveMenuLabels {
                linked_screens: &[],
                mode_images: &serde_json::json!({"requested": 0, "staged": 0}),
                source: &crate::gfx_localization::menu_labels(
                    None,
                    None,
                    &["@ui_STARTGAME".to_string()],
                ),
                text: &crate::gfx_menu_text::TextPatch::default(),
                imported_text: &ImportedTextCounts::default(),
                render_effects: &serde_json::json!({"status":"No source rendering effects verified"}),
            },
        )
        .unwrap();
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(folder.path().join("preview-menu.json")).unwrap())
                .unwrap();
        assert_eq!(report["commands"][0]["label"], "@ui_STARTGAME");
        assert!(report["linked_screens"].as_array().unwrap().is_empty());
        assert_eq!(report["background_video"]["playable"], false);
        assert_eq!(report["authored_flash_text"]["localized"], 0);
        assert_eq!(report["localization"]["unresolved"][0], "ui_startgame");
        assert_eq!(
            report["initialization"]["native_cryengine_host_available"],
            false
        );
        assert_eq!(
            report["initialization"]["verified_transition_to_main_menu"],
            false
        );
        assert_eq!(report["initialization"]["verified_open_screen"], false);
        assert_eq!(report["initialization"]["verified_hide_signing_in"], true);
        assert_eq!(report["initialization"]["verified_hide_patch_notes"], false);
        assert_eq!(report["commands"][0]["command"], "Game_Start");
        assert_eq!(report["commands"][0]["child_submenu"], "Extras");
        assert_eq!(report["resources"]["restored_dds_tiles"], 5);
        assert_eq!(report["fonts"][0]["staged"], false);
        assert_eq!(report["fonts"][0]["staged"], false);
        assert_eq!(
            report["resources"]["instrumented_original_button_conditions"],
            2
        );
        assert_eq!(report["resources"]["skipped_dds_tiles"], 3);
        assert_eq!(report["resources"]["missing_imported_movies"], 1);
        assert_eq!(
            report["resources"]["unresolved_imports"][0],
            "gfxfontlib_locfont.gfx"
        );
        // Source UI_Data commands remain separate from the allowlisted
        // ActionScript callback dispatcher. Native game commands cannot run.
        for preset in [
            PreviewPreset::ShowEngagement,
            PreviewPreset::HideEngagement,
            PreviewPreset::Back,
        ] {
            assert_ne!(preset.script_call().unwrap().0, "Game_Start");
        }
    }
    #[test]
    fn traces_original_button_actions_without_displacing_authored_bytecode() {
        let mut movie = menu_fixture();
        let show = (1u16 << 6).to_le_bytes();
        let at = movie.windows(2).rposition(|window| window == show).unwrap();
        let button = [
            17, 0, // original character ID
            0, // TrackAsMenu
            3, 0, // ActionOffset, measured from this field
            0, // terminating ButtonRecord
            0, 0, // final ButtonCondActionSize
            1, 0, // original event condition
            0x96, 3, 0, 0, b'x', 0, // original ActionPush("x")
            0x26, 0, // original trace and EndAction
        ];
        let mut tag = ((34u16 << 6) | 63).to_le_bytes().to_vec();
        tag.extend_from_slice(&(button.len() as u32).to_le_bytes());
        tag.extend_from_slice(&button);
        movie.splice(at..at, tag);
        let final_length = movie.len() as u32;
        movie[4..8].copy_from_slice(&final_length.to_le_bytes());
        let source = movie.clone();
        let (traced, count) = trace_original_button_actions(&movie).unwrap();
        assert_eq!(count, 1);
        assert_eq!(movie, source); // Source byte-identical.
        assert!(
            traced
                .windows(b"evolve-button:17:1".len())
                .any(|w| w == b"evolve-button:17:1")
        );
        assert!(traced.windows(6).any(|w| w == [0x96, 3, 0, 0, b'x', 0]));
        assert_eq!(
            traced.len(),
            u32::from_le_bytes(traced[4..8].try_into().unwrap()) as usize
        );
        let mut corrupted = movie;
        // The source's ActionOffset must be checked before instrumentation.
        let pos = at + 6 + 3;
        corrupted[pos..pos + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(trace_original_button_actions(&corrupted).is_err());
    }

    #[test]
    fn source_effect_inventory_counts_authored_flags_without_claiming_visual_parity() {
        let mut movie = menu_fixture_arity(7);
        // Synthetic PlaceObject3: color-transform, filter-list, blend-mode.
        let payload = [0x08, 0x03, 0, 0];
        let mut tag = ((70u16 << 6) | (payload.len() as u16))
            .to_le_bytes()
            .to_vec();
        tag.extend_from_slice(&payload);
        let at = movie.len() - 2;
        movie.splice(at..at, tag);
        let size = movie.len() as u32;
        movie[4..8].copy_from_slice(&size.to_le_bytes());
        let inventory = source_render_effect_inventory(&movie);
        assert_eq!(inventory["status"], "SWF flags inventoried");
        assert_eq!(inventory["authored_color_transforms"], 1);
        assert_eq!(inventory["authored_filter_lists"], 1);
        assert_eq!(inventory["authored_blend_modes"], 1);
        assert_eq!(inventory["runtime_verified"], false);
        // Do not accept an incomplete or corrupt authored tag stream as proof.
        movie.pop();
        assert_eq!(source_render_effect_inventory(&movie)["partial"], true);
    }

    #[test]
    fn original_font_inventory_preserves_source_name_glyphs_and_flags() {
        let mut movie = menu_fixture_arity(7);
        // Insert a synthetic DefineFont3 before the original SWF end tag.
        // The glyph count is zero: this tests safe metadata extraction only.
        let mut payload = vec![7, 0, 5, 0, 4]; // ID=7, layout+bold, language, name size
        payload.extend_from_slice(b"Test");
        payload.extend_from_slice(&0u16.to_le_bytes());
        let mut tag = ((75u16 << 6) | (payload.len() as u16))
            .to_le_bytes()
            .to_vec();
        tag.extend_from_slice(&payload);
        let at = movie.len() - 2;
        movie.splice(at..at, tag);
        let size = movie.len() as u32;
        movie[4..8].copy_from_slice(&size.to_le_bytes());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("font.swf");
        fs::write(&path, movie).unwrap();
        let source = original_font_inventory(&path).unwrap();
        assert_eq!(source.len(), 1);
        assert_eq!(source[0]["font_id"], 7);
        assert_eq!(source[0]["font_name"], "Test");
        assert_eq!(source[0]["glyphs"], 0);
        assert_eq!(source[0]["bold"], true);
        assert_eq!(source[0]["italic"], false);
        assert_eq!(original_font_definitions(&path), Some(1));
        // The importer can preserve a compressed CWS wrapper. Inventory
        // must decode that original format without rewriting game sources.
        let raw = fs::read(&path).unwrap();
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        use std::io::Write;
        encoder.write_all(&raw[8..]).unwrap();
        let mut compressed = b"CWS".to_vec();
        compressed.extend_from_slice(&raw[3..8]);
        compressed.extend(encoder.finish().unwrap());
        let compressed_path = directory.path().join("font-compressed.swf");
        fs::write(&compressed_path, compressed).unwrap();
        assert_eq!(original_font_definitions(&compressed_path), Some(1));
        assert_eq!(original_font_inventory(&compressed_path).unwrap(), source);
    }

    #[test]
    fn seven_argument_menu_uses_original_population_and_lifecycle() {
        let original = menu_fixture_arity(7);
        let (patched, allowed) =
            install_live_callbacks(&original, "Libs/UI/main_menu.gfx").unwrap();
        assert!(allowed.contains(&PreviewPreset::OpenScreen));
        for name in [
            "menuButtonData",
            "menuLabels",
            "MODE_NAME_TRAINING",
            "evolvePreview_pushSourceMenu",
            "evolveCallback_pushSourceMenu",
            "evolvePreview_probeImageButtons",
            "evolvePreview_probeMenuRows",
        ] {
            assert!(patched.windows(name.len()).any(|w| w == name.as_bytes()));
        }
        for unsupported in [2, 6, 8] {
            let (_, allowed) =
                install_live_callbacks(&menu_fixture_arity(unsupported), "Libs/UI/main_menu.gfx")
                    .unwrap();
            assert!(!allowed.contains(&PreviewPreset::OpenScreen));
        }
        // The wrapper used to invoke transitionToMainMenu straight after
        // creating the original menu, which could immediately hide it.
        let name = b"evolveCallback_openScreen\0";
        let offset = patched
            .windows(name.len())
            .position(|bytes| bytes == name)
            .unwrap()
            + name.len();
        assert_eq!(&patched[offset..offset + 2], &2u16.to_le_bytes());
        let names = b"menuData\0menuLabels\0";
        assert_eq!(&patched[offset + 2..offset + 2 + names.len()], names);
        let size_at = offset + 2 + names.len();
        let length = usize::from(u16::from_le_bytes(
            patched[size_at..size_at + 2].try_into().unwrap(),
        ));
        let body = &patched[size_at + 2..size_at + 2 + length];
        assert!(
            body.windows(b"pushMenuState".len())
                .any(|w| w == b"pushMenuState")
        );
        assert!(
            !body
                .windows(b"transitionToMainMenu".len())
                .any(|w| w == b"transitionToMainMenu")
        );
        assert_eq!(original, menu_fixture_arity(7));
    }

    #[test]
    #[ignore = "read-only local game acceptance; set EVOLVE_GFX_SOURCE and EVOLVE_GFX_PREVIEW_OUT"]
    fn real_menu_bootstrap_acceptance() {
        let source = std::env::var("EVOLVE_GFX_SOURCE").unwrap();
        let output = std::env::var("EVOLVE_GFX_PREVIEW_OUT").unwrap();
        let bytes = fs::read(&source).unwrap();
        let movie = flash_movie(&bytes).unwrap();
        let (patched, allowed) = install_live_callbacks(&movie, "Libs/UI/main_menu.gfx").unwrap();
        assert!(allowed.contains(&PreviewPreset::OpenScreen));
        use std::io::Write;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .unwrap()
            .write_all(&patched)
            .unwrap();
        assert_eq!(fs::read(source).unwrap(), bytes);
    }

    #[test]
    fn verified_live_callbacks_use_registered_original_handlers_without_restarting() {
        let movie = menu_fixture();
        let (patched, allowed) = install_live_callbacks(&movie, "Libs/UI/main_menu.gfx").unwrap();
        assert!(patched.len() > movie.len());
        assert!(allowed.contains(&PreviewPreset::OpenScreen));
        assert!(allowed.contains(&PreviewPreset::ShowEngagement));
        assert!(allowed.contains(&PreviewPreset::HideEngagement));
        assert!(allowed.contains(&PreviewPreset::Back));
        assert!(!allowed.contains(&PreviewPreset::HideSigningIn));
        assert_eq!(&patched[4..8], &(patched.len() as u32).to_le_bytes());
        assert!(
            patched
                .windows(b"evolvePreview_showEngagementPrompt".len())
                .any(|chunk| chunk == b"evolvePreview_showEngagementPrompt")
        );
        assert!(
            patched
                .windows(b"addCallback".len())
                .any(|chunk| chunk == b"addCallback")
        );
        assert_eq!(movie, menu_fixture());
        let (other, empty) = install_live_callbacks(&movie, "Other.gfx").unwrap();
        assert_eq!(movie, other);
        assert!(empty.is_empty());
    }

    #[test]
    fn verified_original_handler_call_is_isolated_and_rejects_bad_arity() {
        let movie = menu_fixture();
        assert_eq!(
            inject_preview_call(&movie, "Libs/UI/main_menu.gfx", PreviewPreset::AsAuthored)
                .unwrap(),
            movie
        );
        for preset in [
            PreviewPreset::HideEngagement,
            PreviewPreset::ShowEngagement,
            PreviewPreset::ReturnToMainMenu,
            PreviewPreset::Back,
        ] {
            let edited = inject_preview_call(&movie, "Libs/UI/main_menu.gfx", preset).unwrap();
            assert!(edited.len() > movie.len());
            assert_eq!(&edited[4..8], &(edited.len() as u32).to_le_bytes());
            // The injected ActionPush must leave the function name at the top
            // of the stack: [...optional bool, count, "function"].
            let name = preset.script_call().unwrap().0.as_bytes();
            let mut expected_suffix = name.to_vec();
            expected_suffix.extend_from_slice(&[0, 0x3d, 0x17, 0]);
            assert!(
                edited
                    .windows(expected_suffix.len())
                    .any(|window| window == expected_suffix)
            );
            assert_eq!(movie, menu_fixture()); // Source remains byte-identical.
        }
        assert!(inject_preview_call(&movie, "Other.gfx", PreviewPreset::HideEngagement).is_err());
        assert!(
            inject_preview_call(
                &movie,
                "Libs/UI/main_menu.gfx",
                PreviewPreset::HideSigningIn
            )
            .is_err()
        );
        // Conflicting signatures must not pass merely because the function
        // name appears as an unrelated text constant.
        assert!(
            inject_preview_call(
                &movie,
                "Libs/UI/main_menu.gfx",
                PreviewPreset::HidePatchNotes
            )
            .is_err()
        );
    }
    #[test]
    fn textual_function_name_alone_does_not_enable_callback() {
        let action = b"\x96\x12\x00\x00showPatchNotes\x00";
        assert_eq!(defined_function_arity(action, "showPatchNotes"), None);
        assert_eq!(defined_function_arity(&[0x9b, 0x01, 0x00, 0], "a"), None);
    }
    #[test]
    fn nested_imports_keep_original_relative_parent_and_reject_escape() {
        assert_eq!(
            relative_import("", "fonts/main.gfx").unwrap(),
            "fonts/main.gfx"
        );
        assert_eq!(
            relative_import("fonts", "glyphs.gfx").unwrap(),
            "fonts/glyphs.gfx"
        );
        assert_eq!(
            relative_import("ui/icons", "main.swf").unwrap(),
            "ui/icons/main.swf"
        );
        assert!(relative_import("../ui", "font.gfx").is_err());
        assert!(relative_import("fonts", "../hidden.gfx").is_err());
        assert!(relative_import("icons", "C:/private.swf").is_err());
    }
    #[test]
    fn refuses_untrusted_runtimes_and_import_paths() {
        assert!(executable("ruffle.exe").is_err());
        assert!(!safe_import("../ui.swf"));
        assert!(!safe_import("C:/evil.gfx"));
        assert!(safe_import("effects/modes.swf"));
        assert!(!safe_import("image.dds"));
        assert!(safe_import("movie.swf")); // Reject at staging: destination collision.
        assert_eq!("movie.swf".to_ascii_lowercase(), "movie.swf");
    }
    #[test]
    fn valid_gfx_keeps_bytecode_and_tags_when_converted_for_runtime() {
        // Minimal SWF frame header and one DoAction tag, deliberately without
        // invoking a player or requiring copyrighted game files.
        let mut bits = [0u8; 5 + 4 * 17];
        let mut put = |offset: usize, width: usize, value: u32| {
            for n in 0..width {
                bits[offset + n] = ((value >> (width - 1 - n)) & 1) as u8;
            }
        };
        put(0, 5, 17);
        for (i, number) in [0i32, 38400, 0, 21600].iter().enumerate() {
            put(5 + i * 17, 17, *number as u32);
        }
        let mut body = vec![0u8; bits.len().div_ceil(8)];
        for (i, bit) in bits.iter().enumerate() {
            body[i / 8] |= bit << (7 - i % 8);
        }
        body.extend(7680u16.to_le_bytes());
        body.extend(1u16.to_le_bytes());
        body.extend(((12u16 << 6) | 1).to_le_bytes());
        body.push(0); // ActionEnd.
        body.extend(0u16.to_le_bytes());
        let mut original = b"GFX".to_vec();
        original.push(15);
        original.extend(((body.len() + 8) as u32).to_le_bytes());
        original.extend(body);
        let player = flash_movie(&original).unwrap();
        assert_eq!(&player[..3], b"FWS");
        assert_eq!(&player[3..], &original[3..]); // Preserved ActionScript.
    }
    #[test]
    fn rejects_invalid_flash_movie_without_executing_code() {
        assert!(flash_movie(b"FWS\x08").is_err());
        assert!(flash_movie(&[0u8; 24]).is_err());
    }
}
