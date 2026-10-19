//! Private-manager-aligned landing, instructions and informational pages.
//! Home deliberately does not enumerate PAKs or scan the user's installation.
use super::{Message, Page, State};
use iced::widget::{button, column, container, row, scrollable, svg, text};
use iced::{Background, Border, Color, Element, Fill, Length};

const PANEL: Color = Color::from_rgb(0.175, 0.185, 0.202);
const LINE: Color = Color::from_rgb(0.235, 0.245, 0.267);
const MUTED: Color = Color::from_rgb(0.76, 0.79, 0.85);
const ACCENT: Color = Color::from_rgb(0.91, 0.20, 0.30);

fn surface<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content)
        .width(Fill)
        .padding(15)
        .style(|_| container::Style {
            background: Some(Background::Color(PANEL)),
            border: Border {
                color: LINE,
                width: 1.0,
                radius: 10.0.into(),
            },
            ..container::Style::default()
        })
}

fn tile(name: &'static str, description: &'static str, page: Page) -> Element<'static, Message> {
    button(
        row![
            svg(page.icon()).width(24).height(24),
            column![text(name).size(16), text(description).size(12).color(MUTED),].spacing(5),
        ]
        .spacing(14),
    )
    .style(button::secondary)
    .width(Fill)
    .height(Length::Fixed(96.0))
    .padding(15)
    .on_press(Message::Navigate(page))
    .into()
}

fn status_card<'a>(label: &'static str, value: String) -> Element<'a, Message> {
    surface(column![text(label).size(13), text(value).size(12).color(MUTED),].spacing(8)).into()
}

pub(super) fn home(state: &State) -> Element<'_, Message> {
    let hero = surface(column![
        text("EVOLVE STAGE 2  /  MODDING WORKSPACE").size(12).color(ACCENT),
        text("Make something different.").size(32),
        text("A unified workspace for CryEngine assets, visual editing, signed PAK builds, and safe offline testing.").size(13),
        row![
            button("Modding").on_press(Message::Navigate(Page::Files)).style(button::primary),
            button("Asset Library").on_press(Message::Navigate(Page::AssetLibrary)),
            button("Instructions").on_press(Message::Navigate(Page::Instructions)),
        ].spacing(9),
    ].spacing(11));
    let quick = column![
        text("PLAY EVOLVE").size(14),
        text("Choose an offline modded session or launch with original game files.")
            .size(12)
            .color(MUTED),
        row![
            button("Play with Mods").style(button::primary),
            button("Play without Mods").style(button::secondary),
        ]
        .spacing(8),
        text("Launch controls are disabled until game swap and restore integration is verified.")
            .size(12)
            .color(MUTED),
        text("EVOLVE CLIENT").size(14),
        text("Original client executable not yet configured for production launching.")
            .size(12)
            .color(MUTED),
    ]
    .spacing(9);
    let entries = column![
        text("Your workspace").size(19),
        text("Everything in the manager, one click away.")
            .size(12)
            .color(MUTED),
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
            .color(MUTED),
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
            surface(text("System Health   ·   On-demand diagnostics").size(12)),
            row![entries, status].spacing(14),
        ]
        .spacing(20)
        .padding(12),
    )
    .height(Fill)
    .into()
}

fn workflow_step(name: &'static str, description: &'static str) -> Element<'static, Message> {
    surface(column![text(name).size(15), text(description).size(12).color(MUTED),].spacing(7))
        .into()
}

pub(super) fn instructions() -> Element<'static, Message> {
    scrollable(column![
        text("WELCOME TO EVOLVE MOD MANAGER").size(24),
        text("Everything you need to prepare, edit and launch mods in one place.").size(12).color(MUTED),
        surface(column![
            text("STEP 01 / INSTALL EVOLVE STAGE 2").size(15),
            text("Install Evolve Stage 2 through the normal Modded Evolve client from modded-evolve.com before configuring this manager.").size(12).color(MUTED),
            button("Set Up Manager").on_press(Message::Navigate(Page::Settings)).width(Fill),
        ].spacing(8)),
        text("YOUR WORKFLOW").size(13),
        workflow_step("1  Set up your installation", "Choose the installed EvolveGame folder, client executable and private signing stage. The Rust release does not yet integrate production launches."),
        workflow_step("2  Unpack and edit PAKs", "From Modding, inspect archives, browse files, edit supported assets and sign a new offline PAK."),
        workflow_step("3  Play offline with mods", "Launch is only available after installed-game swap and crash recovery are verified."),
        workflow_step("4  Restore original game files", "After testing, original backups must be recovered safely. Isolated staging does not replace production game recovery."),
        text("Keep the entire manager data folder, signing keys, projects and original backups when updating.").size(12).color(MUTED),
        button("Go to Modding  →").on_press(Message::Navigate(Page::Files)).width(Fill),
    ].spacing(12).padding(12)).height(Fill).into()
}

pub(super) fn whats_new() -> Element<'static, Message> {
    scrollable(column![
        text("What's New").size(24),
        surface(column![
            text("Rust rewrite / Development Preview").size(17),
            text("Native Iced/wgpu separate asset viewer, read-only explorer, GPU viewport and a compact timeline.").size(13),
            text("Signed PAK staging, hash-checked recovery, authored DDS material references and limited Blender exports are under validation.").size(13),
            text("Not equivalent to the private v1.3.5 feature set; the public 1.0 release remains blocked.").size(12).color(MUTED),
        ].spacing(9)),
        surface(column![
            text("Private manager reference — v1.3.5").size(17),
            text("Complete-character FBX assembly, source-faithful UV mapping and animation compatibility are the parity targets.").size(13),
            text("Read the project parity checklist for acceptance status; private release notes are not public Rust feature claims.").size(12).color(MUTED),
        ].spacing(9)),
    ].spacing(12).padding(12)).height(Fill).into()
}

pub(super) fn credits() -> Element<'static, Message> {
    scrollable(column![
        text("Credits").size(24),
        surface(column![
            text("@CorvoDL on Discord").size(16),
            text("Project by @CorvoDL").size(13).color(MUTED),
            text("PROJECT REPOSITORY").size(12),
            text("github.com/corvodl/Evolve-Mod-Manager").size(13),
            text("OFFICIAL GAME INSTALLER").size(12),
            text("modded-evolve.com").size(13),
        ].spacing(10)),
        surface(column![
            text("Interface icons").size(16),
            text("Private manager navigation icon design. Flaticon Uicons attribution retained in the original project.").size(12).color(MUTED),
        ].spacing(8)),
        text("Install Evolve Stage 2 via modded-evolve.com before using the manager.").size(12).color(MUTED),
    ].spacing(12).padding(12)).height(Fill).into()
}

pub(super) fn developer(state: &State) -> Element<'_, Message> {
    if state.developer_unlocked {
        column![
            text("Developer").size(24),
            surface(column![
                text("Authorized developer access").size(17),
                text("Private diagnostic and memory tooling has not been ported to this Rust build.").size(13).color(MUTED),
                button("Lock developer area").on_press(Message::LockDeveloper),
            ].spacing(12)),
        ].spacing(13).padding(12).into()
    } else {
        column![
            text("Developer").size(24),
            surface(
                column![
                    text("Password protected").size(17),
                    text("Developer features are not available without authorized credentials.")
                        .size(13)
                        .color(MUTED),
                    text_input("Developer password", &state.developer_password)
                        .secure(true)
                        .on_input(Message::DeveloperPasswordChanged)
                        .on_submit(Message::UnlockDeveloper),
                    button("Unlock").on_press(Message::UnlockDeveloper),
                    text(state.developer_status.as_str()).size(12).color(MUTED),
                ]
                .spacing(12)
            ),
        ]
        .spacing(13)
        .padding(12)
        .into()
    }
}
