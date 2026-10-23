//! Private-manager-aligned landing, instructions and informational pages.
//! Home deliberately does not enumerate PAKs or scan the user's installation.
use super::{Message, Page, State};
use crate::design;
use iced::widget::{button, column, container, row, scrollable, svg, text};
use iced::{Background, Border, Element, Fill, Length};

fn surface<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content)
        .width(Fill)
        .padding(15)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::surface(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: 10.0.into(),
            },
            ..container::Style::default()
        })
}

fn tile(name: &'static str, description: &'static str, page: Page) -> Element<'static, Message> {
    design::button(
        row![
            svg(page.icon()).style(design::icon).width(24).height(24),
            column![
                text(name).size(16),
                text(description).size(12).style(design::muted),
            ]
            .spacing(5),
        ]
        .spacing(14)
        .align_y(iced::Center),
    )
    .style(|theme, status| {
        let mut style = design::secondary(theme, status);
        style.background = Some(Background::Color(match status {
            button::Status::Hovered => design::field(theme),
            button::Status::Pressed => design::selection(theme),
            _ => design::surface(theme),
        }));
        style.text_color = theme.palette().text;
        style.border = Border {
            color: design::border(theme),
            width: 1.0,
            radius: 12.0.into(),
        };
        style
    })
    .width(Fill)
    .height(Length::Fixed(106.0))
    .padding(15)
    .on_press(Message::Navigate(page))
    .into()
}

fn status_card<'a>(label: &'static str, value: String) -> Element<'a, Message> {
    surface(
        column![
            text(label).size(13),
            text(value).size(12).style(design::muted),
        ]
        .spacing(8),
    )
    .into()
}

fn health_entry<'a>(title: &'static str, value: &str) -> Element<'a, Message> {
    surface(
        column![
            text(title).size(13),
            text(value.to_owned()).size(12).style(design::muted),
        ]
        .spacing(6),
    )
    .into()
}

fn health_panel(state: &State) -> Element<'_, Message> {
    let heading = design::button(if state.health_expanded {
        "⌄ System Health   ·   On-demand diagnostics"
    } else {
        "› System Health   ·   On-demand diagnostics"
    })
    .on_press(Message::ToggleSystemHealth)
    .style(design::secondary)
    .width(Fill);
    if !state.health_expanded {
        return heading.into();
    }
    let check = row![
        design::button(if state.health_checking {
            "Checking..."
        } else {
            "Check System Health"
        })
        .on_press_maybe((!state.health_checking).then_some(Message::CheckSystemHealth)),
        text("Read-only until you choose an explicit operation")
            .size(12)
            .style(design::muted),
    ]
    .spacing(12);
    if let Some(result) = &state.health_report {
        column![
            heading,
            text("Checks are opt-in. Firewall, registry, dumps and cleanup settings are never changed.")
                .size(12).style(design::muted),
            check,
            row![
                health_entry("Windows Firewall", &result.firewall),
                health_entry("Game Process", &result.game_process),
            ].spacing(10),
            row![
                health_entry("Crash Dumps & Support", &result.dumps),
                health_entry("Storage & Backups", &result.storage),
            ].spacing(10),
            health_entry("Game Environment", &result.environment),
            health_entry("Recovery & Projects", &result.recovery),
        ].spacing(10).into()
    } else {
        column![
            heading,
            text("Nothing is scanned while this panel is closed. Select Check System Health to run read-only checks.")
                .size(12).style(design::muted),
            check,
        ].spacing(9).into()
    }
}

pub(super) fn home(state: &State) -> Element<'_, Message> {
    let hero = surface(column![
        text("EVOLVE STAGE 2  /  MODDING WORKSPACE").size(12).style(design::accent),
        text("Make something different.").size(32),
        text("A unified workspace for CryEngine assets, visual editing, signed PAK builds, and safe offline testing.").size(13),
        row![
            design::button("Modding").on_press(Message::Navigate(Page::Files)).style(design::primary),
            design::button("Asset Library").on_press(Message::Navigate(Page::AssetLibrary)).style(design::secondary),
            design::button("Instructions").on_press(Message::Navigate(Page::Instructions)).style(design::secondary),
        ].spacing(9),
    ].spacing(11)).padding([25, 28]).style(|theme| container::Style {
        background: Some(Background::Color(design::surface(theme))),
        border: Border { color: design::border(theme), width: 1.0, radius: 16.0.into() },
        ..container::Style::default()
    });
    let quick = column![
        text("PLAY EVOLVE").size(14),
        text("Choose an offline modded session or launch with original game files.")
            .size(12)
            .style(design::muted),
        row![
            design::button("Play with Mods").style(design::primary),
            design::button("Play without Mods").style(design::secondary),
        ]
        .spacing(8),
        text("Launch controls are disabled until game swap and restore integration is verified.")
            .size(12)
            .style(design::muted),
    ]
    .spacing(9);
    let entries = column![
        text("Your workspace").size(19),
        text("Everything in the manager, one click away.")
            .size(12)
            .style(design::muted),
        row![
            tile(
                "Modding",
                "Open a project, edit assets and sign a PAK.",
                Page::Files
            ),
            tile(
                "Asset Library",
                "Browse textures, models, animation and GFX.",
                Page::AssetLibrary
            ),
        ]
        .spacing(10),
        row![
            tile(
                "Instructions",
                "First-time setup and safe modding workflow.",
                Page::Instructions
            ),
            tile(
                "Settings",
                "Appearance, paths, updates and maintenance.",
                Page::Settings
            ),
        ]
        .spacing(10),
        row![
            tile(
                "What's New",
                "Changes included in the latest release.",
                Page::WhatsNew
            ),
            tile(
                "Credits",
                "Project information and acknowledgments.",
                Page::Credits
            ),
        ]
        .spacing(10),
    ]
    .spacing(9)
    .width(Length::FillPortion(5));
    let status = column![
        text("At a glance").size(19),
        text("Installed game and client status; launch actions are separate.")
            .size(12)
            .style(design::muted),
        status_card("Game installation", "Not configured for launch".to_owned()),
        status_card("Game version", "Not detected".to_owned()),
        status_card(
            "Signing stage",
            if state.preferences.stage.is_empty() {
                "Not prepared".to_owned()
            } else {
                "Isolated offline stage configured".to_owned()
            }
        ),
        status_card(
            "Editing project",
            if state.preferences.current_workspace.is_empty() {
                "None selected".to_owned()
            } else {
                state.preferences.current_workspace.clone()
            }
        ),
    ]
    .spacing(9)
    .width(Length::FillPortion(2));
    scrollable(
        column![
            hero,
            quick,
            health_panel(state),
            row![entries, status].spacing(14),
        ]
        .spacing(20)
        .padding(12),
    )
    .height(Fill)
    .into()
}

fn workflow_step(name: &'static str, description: &'static str) -> Element<'static, Message> {
    surface(
        column![
            text(name).size(15),
            text(description).size(12).style(design::muted),
        ]
        .spacing(7),
    )
    .into()
}

pub(super) fn instructions() -> Element<'static, Message> {
    scrollable(column![
        text("WELCOME TO EVOLVE MOD MANAGER").size(24),
        text("Everything you need to prepare, edit and launch mods in one place.").size(12).style(design::muted),
        surface(column![
            text("STEP 01 / INSTALL EVOLVE STAGE 2").size(15),
            text("Install Evolve Stage 2 through the normal Modded Evolve client from modded-evolve.com before configuring this manager.").size(12).style(design::muted),
            design::button("Set Up Manager").on_press(Message::Navigate(Page::Settings)).width(Fill),
        ].spacing(8)),
        text("YOUR WORKFLOW").size(13),
        workflow_step("1  Set up your installation", "Choose the installed EvolveGame folder, client executable and private signing stage. The Rust release does not yet integrate production launches."),
        workflow_step("2  Unpack and edit PAKs", "From Modding, inspect archives, browse files, edit supported assets and sign a new offline PAK."),
        workflow_step("3  Play offline with mods", "Launch is only available after installed-game swap and crash recovery are verified."),
        workflow_step("4  Restore original game files", "After testing, original backups must be recovered safely. Isolated staging does not replace production game recovery."),
        text("Keep the entire manager data folder, signing keys, projects and original backups when updating.").size(12).style(design::muted),
        design::button("Go to Modding  →").on_press(Message::Navigate(Page::Files)).width(Fill),
    ].spacing(12).padding(12)).height(Fill).into()
}

pub(super) fn whats_new() -> Element<'static, Message> {
    scrollable(column![
        text("What's New").size(24),
        surface(column![
            text("Rust rewrite / Development Preview").size(17),
            text("Native Iced/wgpu separate asset viewer, read-only explorer, GPU viewport and a compact timeline.").size(13),
            text("Signed PAK staging, hash-checked recovery, authored DDS material references and limited Blender exports are under validation.").size(13),
            text("Not equivalent to the private v1.3.5 feature set; the public 1.0 release remains blocked.").size(12).style(design::muted),
        ].spacing(9)),
        surface(column![
            text("Private manager reference — v1.3.5").size(17),
            text("Complete-character FBX assembly, source-faithful UV mapping and animation compatibility are the parity targets.").size(13),
            text("Read the project parity checklist for acceptance status; private release notes are not public Rust feature claims.").size(12).style(design::muted),
        ].spacing(9)),
    ].spacing(12).padding(12)).height(Fill).into()
}

pub(super) fn credits() -> Element<'static, Message> {
    scrollable(column![
        text("Credits").size(24),
        surface(column![
            text("@CorvoDL on Discord").size(16),
            text("Project by @CorvoDL").size(13).style(design::muted),
            text("PROJECT REPOSITORY").size(12),
            text("github.com/corvodl/Evolve-Mod-Manager").size(13),
            text("OFFICIAL GAME INSTALLER").size(12),
            text("modded-evolve.com").size(13),
        ].spacing(10)),
        surface(column![
            text("Interface icons").size(16),
            text("Private manager navigation icon design. Flaticon Uicons attribution retained in the original project.").size(12).style(design::muted),
        ].spacing(8)),
        text("Install Evolve Stage 2 via modded-evolve.com before using the manager.").size(12).style(design::muted),
    ].spacing(12).padding(12)).height(Fill).into()
}

pub(super) fn developer(state: &State) -> Element<'_, Message> {
    if state.developer_unlocked {
        scrollable(column![
            text("Developer").size(24),
            surface(column![
                text("Authorized developer access").size(17),
                text("Phase 1  /  External memory inspector").size(16),
                text("Injection-free, read-only. Detect one Evolve.exe process on demand; inspect 32 bytes at an explicit hex address or a validated pointer chain. Offsets are not bundled. Process identity and offline status cannot yet be independently verified.").size(12).style(design::muted),
                design::button(if state.runtime_checking { "Inspecting..." } else { "Detect Evolve process" })
                    .on_press_maybe((!state.runtime_checking).then_some(Message::RuntimeDetect)),
                text(state.runtime_snapshot.as_ref().map(|s| s.status.as_str()).unwrap_or("Not scanned")).size(12).style(design::muted),
                text(state.runtime_snapshot.as_ref().map(|s| s.executable.as_str()).unwrap_or("Executable not checked")).size(12).style(design::muted),
                design::text_input("Address (0x... or Evolve.exe+0x...)", &state.runtime_address).on_input(Message::RuntimeAddressChanged),
                design::text_input("Pointer offsets (optional: 0x10,0x20)", &state.runtime_offsets).on_input(Message::RuntimeOffsetsChanged),
                design::button("Read 32 bytes")
                    .on_press_maybe((!state.runtime_checking && state.runtime_snapshot.as_ref().and_then(|s| s.pid).is_some()).then_some(Message::RuntimeRead)),
                text(state.runtime_result.as_str()).size(12).style(design::muted),
                text("Stage 2 Goliath armor / guided Float32 finder").size(16),
                text("Read-only full-process search for values near the observed internal full-armor scale of ~1000. Requires full armor at the first scan. No debugger, pointer scan, injection, or memory writes. Match verification is behavioral, not an actor identity guarantee.").size(12).style(design::muted),
                iced::widget::checkbox(state.armor_offline_confirmed).label("I am in a local OFFLINE match; disable when online").on_toggle(Message::ArmorOfflineChanged),
                design::button("Find full-armor Float32 candidates")
                    .on_press_maybe((state.armor_offline_confirmed && !state.runtime_checking && state.runtime_snapshot.as_ref().and_then(|s|s.pid).is_some()).then_some(Message::ArmorBegin)),
                text("After the scan: lose armor and press Damage; stand with constant armor and press Stable; feed to restore armor and press Feed. Keep armor below maximum when feeding.").size(12).style(design::muted),
                row![
                    design::button("Damage").on_press_maybe((state.armor_offline_confirmed && !state.runtime_checking && state.armor_scan.is_some()).then_some(Message::ArmorRefine(super::runtime_armor::Step::Damage))),
                    design::button("Stable").on_press_maybe((state.armor_offline_confirmed && !state.runtime_checking && state.armor_scan.is_some()).then_some(Message::ArmorRefine(super::runtime_armor::Step::Stable))),
                    design::button("Feed").on_press_maybe((state.armor_offline_confirmed && !state.runtime_checking && state.armor_scan.is_some()).then_some(Message::ArmorRefine(super::runtime_armor::Step::Feed))),
                ].spacing(8),
                text(state.armor_report.as_str()).size(12).style(design::muted),
                text(state.armor_live.map(|v|format!("Live armor (internal units): {v:.2}")).unwrap_or_else(||"Live armor: awaiting verified candidate".into())).size(13),
                text("Health candidates  /  Read-only").size(16),
                text("Uses the hexadecimal address above to inspect one containing 4 KiB page. It cannot identify actors or find health across the entire game yet. Optional expected HP narrows initial candidates; blank starts an unknown-value scan. Use only in an offline match.").size(12).style(design::muted),
                design::text_input("Expected health (optional, e.g. 1500)", &state.runtime_health_value)
                    .on_input(Message::RuntimeHealthValueChanged),
                row![
                    design::button("Find Float candidates")
                        .on_press_maybe((!state.runtime_checking && state.runtime_snapshot.as_ref().and_then(|s| s.pid).is_some()).then_some(Message::RuntimeHealthStart(super::runtime_health::Kind::Float))),
                    design::button("Find 4-Byte candidates")
                        .on_press_maybe((!state.runtime_checking && state.runtime_snapshot.as_ref().and_then(|s| s.pid).is_some()).then_some(Message::RuntimeHealthStart(super::runtime_health::Kind::Integer))),
                ].spacing(8),
                text("Take damage in-game, then narrow candidates with Decreased. Use Unchanged when no damage occurred. Candidates are NOT confirmed health or actors.").size(12).style(design::muted),
                row![
                    design::button("Decreased")
                        .on_press_maybe((!state.runtime_checking && state.runtime_health_scan.is_some()).then_some(Message::RuntimeHealthRefine(super::runtime_health::Filter::Decreased))),
                    design::button("Unchanged")
                        .on_press_maybe((!state.runtime_checking && state.runtime_health_scan.is_some()).then_some(Message::RuntimeHealthRefine(super::runtime_health::Filter::Unchanged))),
                    design::button("Increased")
                        .on_press_maybe((!state.runtime_checking && state.runtime_health_scan.is_some()).then_some(Message::RuntimeHealthRefine(super::runtime_health::Filter::Increased))),
                    design::button("Changed")
                        .on_press_maybe((!state.runtime_checking && state.runtime_health_scan.is_some()).then_some(Message::RuntimeHealthRefine(super::runtime_health::Filter::Changed))),
                ].spacing(8),
                text(state.runtime_health_report.as_str()).size(12).style(design::muted),
                text("Phase 2  /  Offline runtime controls").size(16),
                text("Preview typed field changes against a manually entered address above. Health/cooldowns are f32, teleport is X,Y,Z and weapon reference is 0x... . These are BYTE PREVIEWS ONLY; live edits remain locked until verified offline and exact-build actor layouts are available.").size(12).style(design::muted),
                design::text_input("Desired value (e.g. 100000 / 0 / 1,2,3 / 0x10008)", &state.runtime_phase2_value)
                    .on_input(Message::RuntimePhase2ValueChanged),
                row![
                    design::button("Preview health").on_press(Message::PreviewRuntimePhase2(super::runtime_experiments::Phase2Action::Health)),
                    design::button("Preview cooldown").on_press(Message::PreviewRuntimePhase2(super::runtime_experiments::Phase2Action::Cooldown)),
                    design::button("Preview teleport").on_press(Message::PreviewRuntimePhase2(super::runtime_experiments::Phase2Action::Teleport)),
                    design::button("Preview weapon").on_press(Message::PreviewRuntimePhase2(super::runtime_experiments::Phase2Action::Equip)),
                ].spacing(8),
                text("Phase 3  /  Advanced engine operations").size(16),
                text("Preview candidate engine class operations. Spawns accept ClassName,Count (up to 32). Character swaps and abilities accept a single identifier. This validates inputs only: no injection, entity creation, native function execution, or RMI calls.").size(12).style(design::muted),
                design::text_input("Class/ability identifier (e.g. Goliath,2)", &state.runtime_phase3_value)
                    .on_input(Message::RuntimePhase3ValueChanged),
                row![
                    design::button("Preview spawn").on_press(Message::PreviewRuntimePhase3(super::runtime_experiments::Phase3Action::Spawn)),
                    design::button("Preview character").on_press(Message::PreviewRuntimePhase3(super::runtime_experiments::Phase3Action::SwapCharacter)),
                    design::button("Preview ability").on_press(Message::PreviewRuntimePhase3(super::runtime_experiments::Phase3Action::Ability)),
                ].spacing(8),
                text("Engine signature probe / read-only").size(16),
                text("Search exactly 4 KiB from the manual address above for a Ghidra-style byte signature (AA BB ?? CC). Candidate addresses require verification in the actual game build.").size(12).style(design::muted),
                design::text_input("AOB signature, e.g. 48 8B ?? ?? 89", &state.runtime_signature)
                    .on_input(Message::RuntimeSignatureChanged),
                design::button("Scan 4 KiB for signature")
                    .on_press_maybe((!state.runtime_checking && state.runtime_snapshot.as_ref().and_then(|s| s.pid).is_some()).then_some(Message::RuntimeScanSignature)),
                text(state.runtime_plan_result.as_str()).size(12).style(design::muted),
                design::button("Lock developer area").on_press(Message::LockDeveloper),
            ].spacing(12)),
        ].spacing(13).padding(12)).height(Fill).into()
    } else {
        column![
            text("Developer").size(24),
            surface(
                column![
                    text("Password protected").size(17),
                    text("Developer features are not available without authorized credentials.")
                        .size(13)
                        .style(design::muted),
                    design::text_input("Developer password", &state.developer_password)
                        .secure(true)
                        .on_input(Message::DeveloperPasswordChanged)
                        .on_submit(Message::UnlockDeveloper),
                    design::button("Unlock").on_press(Message::UnlockDeveloper),
                    text(state.developer_status.as_str())
                        .size(12)
                        .style(design::muted),
                ]
                .spacing(12)
            ),
        ]
        .spacing(13)
        .padding(12)
        .into()
    }
}
