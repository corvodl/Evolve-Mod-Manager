//! Experimental interactive GFX preview using an independently installed Ruffle
//! desktop runtime. The original ActionScript bytecode remains in the movie;
//! only supported Scaleform DDS image tags are reconstructed for rendering.
//! This is intentionally NOT a CryEngine host callback implementation.
use crate::{gfx_editor::Session, gfx_external_textures, gfx_live_web};
use evolve_core::gfx::{MAX_DECOMPRESSED, MAX_SOURCE, inspect_gfx};
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
        if found.is_empty() || found.iter().any(|&count| count != expected) {
            continue;
        }
        let method = format!("evolvePreview_{}", function);
        let wrapper = format!("evolveCallback_{}", function);
        // DefineFunction(wrapper, optional boolean parameter) body.
        let mut body = Vec::new();
        if argument.is_some() {
            avm1_push_string(&mut body, "value")?;
            body.push(0x1c); // GetVariable
        }
        avm1_push_i32(&mut body, expected as i32);
        avm1_push_string(&mut body, function)?;
        body.extend_from_slice(&[0x3d, 0x17]); // CallFunction, Pop result
        avm1_push_boolean(&mut body, true);
        body.extend_from_slice(&[0x3e, 0]); // Return(true), End
        let mut def = Vec::new();
        def.extend_from_slice(wrapper.as_bytes());
        def.push(0);
        def.extend_from_slice(&(expected as u16).to_le_bytes());
        if expected == 1 {
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
        allowed.push(*preset);
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
) -> Result<(Vec<u8>, usize, usize), String> {
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
    ))
}
fn stage_import(
    session: &Session,
    virtual_movie: &str,
    reference: &str,
    output: &Path,
) -> Result<Option<(usize, usize)>, String> {
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
    let virtual_path = virtual_movie
        .rsplit_once('/')
        .map(|(parent, _)| format!("{parent}/{reference}"))
        .unwrap_or_else(|| reference.to_owned());
    let (swf, converted, skipped) =
        renderable_movie(&bytes, &virtual_path, &session.source, session)?;
    let target = output.join(reference);
    let parent = target
        .parent()
        .ok_or("Invalid imported movie destination")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    fs::write(target, swf).map_err(|e| e.to_string())?;
    Ok(Some((converted, skipped)))
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
    let (swf, mut converted, mut skipped) =
        renderable_movie(&input, &session.relative, &session.source, session)?;
    let root_movie = root.path().join("movie.swf");
    let movie = inject_preview_call(&swf, &session.relative, preset)?;
    fs::write(&root_movie, movie).map_err(|e| e.to_string())?;
    let mut missing = 0usize;
    for name in session.info.imports.iter().take(MAX_IMPORTS) {
        match stage_import(session, &session.relative, name, root.path()) {
            Ok(Some((images, skip))) => {
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

/// Serialize *source-defined* UI_Data commands for an optional live browser
/// inspector. These do not become fabricated SWF display-list nodes. Commands
/// that require CryEngine remain read-only; only known child submenu links can
/// be traversed locally. This is diagnostics, NOT native menu emulation.
fn write_live_menu_report(
    session: &Session,
    scratch: &Path,
    restored: usize,
    skipped: usize,
    missing: usize,
    traced_buttons: usize,
    verified_actions: &[PreviewPreset],
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
                "hidden_when": item.hidden_when
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
    let label_tokens = session.menu_commands.iter()
        .map(|entry| entry.label.clone()).collect::<Vec<_>>();
    let localization = crate::gfx_localization::menu_labels(
        session.library_root.as_deref(),
        session.source_build.as_deref(),
        &label_tokens,
    );
    // Imported font movies remain original same-build assets; resolving an
    // import does not prove Ruffle supports its Scaleform glyph tags.
    let font_imports = session.info.imports.iter()
        .filter(|name| name.to_ascii_lowercase().contains("font"))
        .map(|name| {
            let found = session.dependencies.iter().find(|entry|
                entry.reference.eq_ignore_ascii_case(name));
            serde_json::json!({
                "movie": name,
                "staged": scratch.join(name).is_file(),
                "source": found.map_or("Not resolved", |dependency| dependency.outcome.as_str())
            })
        }).collect::<Vec<_>>();
    let initialized_in_swf = verified_actions.contains(&PreviewPreset::ReturnToMainMenu);
    let report = serde_json::json!({
        "source": "verified same-build UI_Data/MainMenuScreen.xml",
        "localization": localization,
        "fonts": font_imports,
        "initialization": {
            "verified_transition_to_main_menu": initialized_in_swf,
            "native_cryengine_host_available": false,
            "status": "Only original verified ActionScript handlers may initialize local menu state. Native game state is unavailable."
        },
        "menu_status": session.menu_command_status,
        "commands": entries,
        "resources": {
            "restored_dds_tiles": restored,
            "instrumented_original_button_conditions": traced_buttons,
            "skipped_dds_tiles": skipped,
            "missing_imported_movies": missing,
            "unresolved_imports": unresolved,
            "has_source_build": session.source_build.is_some(),
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
pub fn launch_live(session: &Session, web_runtime: &str) -> Result<RunningPreview, String> {
    let scratch = tempfile::Builder::new()
        .prefix("evolve-gfx-live-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let bytes = fs::read(&session.editable).map_err(|e| e.to_string())?;
    let (swf, mut converted, mut skipped) =
        renderable_movie(&bytes, &session.relative, &session.source, session)?;
    // Trace authored SWF button handlers before registering the original
    // live callbacks. No synthesized UI or native handler mapping is added.
    let (inspected, traced_buttons) = if session
        .relative
        .eq_ignore_ascii_case("Libs/UI/main_menu.gfx")
    {
        trace_original_button_actions(&swf)?
    } else {
        (swf, 0)
    };
    let (movie, actions) = install_live_callbacks(&inspected, &session.relative)?;
    fs::write(scratch.path().join("movie.swf"), movie).map_err(|e| e.to_string())?;
    let mut missing = 0usize;
    for name in session.info.imports.iter().take(MAX_IMPORTS) {
        match stage_import(session, &session.relative, name, scratch.path()) {
            Ok(Some((images, skip))) => {
                converted += images;
                skipped += skip;
            }
            Ok(None) => missing += 1,
            Err(error) => return Err(format!("Imported Scaleform movie {name}: {error}")),
        }
    }
    write_live_menu_report(
        session,
        scratch.path(),
        converted,
        skipped,
        missing,
        traced_buttons,
        &actions,
    )?;
    let live = gfx_live_web::LiveServer::start(scratch, actions, web_runtime)?;
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
    fn menu_fixture() -> Vec<u8> {
        // Synthetic AVM1 DefineFunction root script (no original Evolve assets).
        let mut movie = b"FWS\x0f".to_vec();
        movie.extend_from_slice(&[0; 4]);
        movie.extend_from_slice(&[0x08, 0x00]);
        movie.extend_from_slice(&[0, 30, 1, 0]);
        let mut action = Vec::new();
        for (name, params) in [
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
            }],
            menu_command_status: "1 verified original menu entry".into(),
            library_root: None,
            source_build: None,
            edits: 0,
            applied: false,
        };
        write_live_menu_report(&session, folder.path(), 5, 3, 1, 2, &[PreviewPreset::Back]).unwrap();
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(folder.path().join("preview-menu.json")).unwrap())
                .unwrap();
        assert_eq!(report["commands"][0]["label"], "@ui_STARTGAME");
        assert_eq!(report["localization"]["unresolved"][0], "ui_startgame");
        assert_eq!(report["initialization"]["native_cryengine_host_available"], false);
        assert_eq!(report["initialization"]["verified_transition_to_main_menu"], false);
        assert_eq!(report["commands"][0]["command"], "Game_Start");
        assert_eq!(report["commands"][0]["child_submenu"], "Extras");
        assert_eq!(report["resources"]["restored_dds_tiles"], 5);
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
    fn verified_live_callbacks_use_registered_original_handlers_without_restarting() {
        let movie = menu_fixture();
        let (patched, allowed) = install_live_callbacks(&movie, "Libs/UI/main_menu.gfx").unwrap();
        assert!(patched.len() > movie.len());
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
