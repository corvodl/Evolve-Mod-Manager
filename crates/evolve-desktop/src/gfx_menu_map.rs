//! Read-only command catalog from the *same-build* UI_Data MainMenuScreen.
//! This is a menu binding reference, not a native CryEngine command executor.
use evolve_core::{LibraryReferenceStatus, resolve_imported_reference};
use quick_xml::{Reader, events::Event};
use std::{fs, path::Path};

const MAX_XML: u64 = 256 * 1024;
const MAX_BUTTONS: usize = 128;

#[derive(Clone, Debug)]
pub struct MenuCommand {
    pub submenu: String,
    pub label: String,
    pub command: String,
    pub child_submenu: String,
    pub disabled_when: String,
    pub hidden_when: String,
}

impl MenuCommand {
    pub fn outcome(&self) -> &'static str {
        if !self.child_submenu.is_empty() {
            "Local submenu selection (movie/host transition still needs testing)"
        } else {
            "Game-bound command (not executed by Ruffle)"
        }
    }
}

/// Return UI_Data commands only for a verified exact relative path and source
/// build. Missing or ambiguous definitions are reported, never substituted
/// with hardcoded buttons from another game build.
pub fn load(
    root: Option<&Path>,
    build: Option<&str>,
) -> Result<(Vec<MenuCommand>, String), String> {
    let (Some(root), Some(build)) = (root, build) else {
        return Ok((
            Vec::new(),
            "UI_Data command mapping unavailable without a source-build Asset Library.".into(),
        ));
    };
    if build.trim().is_empty() {
        return Err("Cannot inspect UI_Data from an unnamed build".into());
    }
    match resolve_imported_reference(root, "UI/MainMenuScreen.xml", Some(build))
        .map_err(|e| e.to_string())?
    {
        LibraryReferenceStatus::Missing => Ok((
            Vec::new(),
            "No exact same-build UI/MainMenuScreen.xml found in Asset Library.".into(),
        )),
        LibraryReferenceStatus::Ambiguous(_) => Ok((
            Vec::new(),
            "Multiple same-build UI/MainMenuScreen.xml files; mapping remains unavailable.".into(),
        )),
        LibraryReferenceStatus::Found(info) => {
            let path = Path::new(&info.absolute_path);
            let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            if !meta.file_type().is_file() || meta.len() > MAX_XML {
                return Err("UI_Data mapping is linked, oversized, or not a regular file".into());
            }
            let bytes = fs::read(path).map_err(|e| e.to_string())?;
            let commands = parse(&bytes)?;
            let status = format!(
                "{} original menu command entries · source build {} / {}. Native commands disabled.",
                commands.len(),
                build,
                info.archive_name
            );
            Ok((commands, status))
        }
    }
}

fn parse(input: &[u8]) -> Result<Vec<MenuCommand>, String> {
    if input.is_empty() || input.len() > MAX_XML as usize {
        return Err("UI_Data menu XML is empty or too large".into());
    }
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(true);
    let mut scratch = Vec::new();
    let mut submenu = None::<String>;
    let mut in_main_menu = false;
    let mut main_menu_depth = 0usize;
    let mut commands = Vec::new();
    let mut found_main_menu = false;
    loop {
        match reader.read_event_into(&mut scratch) {
            Ok(Event::Start(e)) => {
                let tag = e.name().as_ref().to_vec();
                if tag == b"MainMenu" && !in_main_menu {
                    found_main_menu = true;
                    in_main_menu = true;
                } else if in_main_menu {
                    // UI_Data can add original submenu groups beyond the four
                    // historically hardcoded names. Keep source-defined child
                    // groups only: nested icon/condition elements are not
                    // independent menus.
                    if main_menu_depth == 0
                        && (2..=64).contains(&tag.len())
                        && tag.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
                        && tag != b"Button"
                    {
                        submenu = Some(String::from_utf8_lossy(&tag).into_owned());
                    }
                    main_menu_depth += 1;
                }
            }
            Ok(Event::Empty(e)) if in_main_menu && e.name().as_ref() == b"Button" => {
                let Some(group) = &submenu else {
                    continue;
                };
                let mut command = MenuCommand {
                    submenu: group.clone(),
                    label: String::new(),
                    command: String::new(),
                    child_submenu: String::new(),
                    disabled_when: String::new(),
                    hidden_when: String::new(),
                };
                for attribute in e.attributes().with_checks(true) {
                    let attribute = attribute.map_err(|_| "Malformed UI_Data button attribute")?;
                    let value = attribute
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|_| "Malformed UI_Data button text")?
                        .into_owned();
                    if value.len() > 512 {
                        return Err("UI_Data menu attribute exceeds size limit".into());
                    }
                    match attribute.key.as_ref() {
                        b"display_name" => command.label = value,
                        b"command" => command.command = value,
                        b"child_submenu" => command.child_submenu = value,
                        b"disabled_when" => command.disabled_when = value,
                        b"hidden_when" => command.hidden_when = value,
                        _ => (),
                    }
                }
                if !command.command.is_empty() {
                    commands.push(command);
                }
                if commands.len() > MAX_BUTTONS {
                    return Err("UI_Data menu has too many commands".into());
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"MainMenu" {
                    in_main_menu = false;
                    main_menu_depth = 0;
                    submenu = None;
                } else if submenu
                    .as_deref()
                    .is_some_and(|name| name.as_bytes() == e.name().as_ref())
                {
                    submenu = None;
                }
                if in_main_menu {
                    main_menu_depth = main_menu_depth.saturating_sub(1);
                }
            }
            Ok(Event::DocType(_)) => {
                return Err("UI_Data menu XML may not declare external entities or DTDs".into());
            }
            Ok(Event::Eof) => break,
            Err(_) => return Err("Malformed UI_Data main-menu XML".into()),
            _ => (),
        }
        scratch.clear();
    }
    if !found_main_menu || commands.is_empty() {
        return Err("UI_Data XML does not contain an original main-menu command table".into());
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_original_screen_style_without_triggering_game_commands() {
        let xml = br#"<ScreenCinematics><MainMenu><Main>
          <Button display_name="@ui_Training" command="Singleplayer" child_submenu="Singleplayer" />
          <Button display_name="@ui_mp_OnlineGame" command="OnlineMultiplayer"
                  disabled_when="CustomCondition=[!TOSAccepted]" />
        </Main><Extras><Button display_name="@ui_PatchNotes" command="patchNotesButton" /></Extras>
        </MainMenu></ScreenCinematics>"#;
        let map = parse(xml).unwrap();
        assert_eq!(map.len(), 3);
        assert_eq!(map[0].submenu, "Main");
        assert_eq!(map[0].child_submenu, "Singleplayer");
        assert!(map[0].outcome().contains("Local submenu"));
        assert_eq!(map[1].command, "OnlineMultiplayer");
        assert!(map[1].outcome().contains("Game-bound"));
        assert_eq!(map[2].submenu, "Extras");
        assert_eq!(map[2].command, "patchNotesButton");
    }

    #[test]
    fn preserves_all_original_ui_data_submenu_groups_not_only_hardcoded_four() {
        let xml = br#"<Root><MainMenu>
            <Main><Button display_name="@ui_Play" command="Browse" child_submenu="Multiplayer"/></Main>
            <Multiplayer><Button display_name="@ui_Join" command="OnlineMultiplayer"/></Multiplayer>
            <Profiles><Button display_name="@ui_Swap" command="Profiles"/></Profiles>
            </MainMenu></Root>"#;
        let commands = parse(xml).unwrap();
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0].submenu, "Main");
        assert_eq!(commands[1].submenu, "Multiplayer");
        assert_eq!(commands[2].submenu, "Profiles");
        assert_eq!(commands[0].child_submenu, "Multiplayer");
    }

    #[test]
    fn refuses_dtd_and_incomplete_menu() {
        assert!(parse(b"<!DOCTYPE a [<!ENTITY external SYSTEM 'file:///x'>]><a/>").is_err());
        assert!(parse(b"<ScreenCinematics />").is_err());
    }
}
