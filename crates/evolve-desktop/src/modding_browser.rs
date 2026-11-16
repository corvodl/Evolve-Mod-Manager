//! The basic Modding page: pick a build, pick a character from a portrait
//! grid, then open its gameplay values or models. Enabled mods stay visible.
use super::{Message, State};
use crate::character_roster::{CHARACTERS, GROUPS};
use crate::ui_kit::{BadgeKind, badge};
use crate::{design, icons, ui_kit};
use evolve_core::character_gameplay::{WILDLIFE, owns_document, owns_wildlife_document};
use iced::widget::{button, column, container, image, row, scrollable, space, text};
use iced::{Alignment, Background, Border, Element, Fill, Length};

const CARD_WIDTH: f32 = 128.0;
const PANEL_WIDTH: f32 = 300.0;

fn card_style(theme: &iced::Theme) -> container::Style {
    design::panel(theme)
}

/// A selectable tile: surface at rest, field on hover, accent outline when chosen.
fn tile_style(selected: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style + 'static {
    move |theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if selected {
                design::selection(theme)
            } else if hovered {
                design::field(theme)
            } else {
                design::surface(theme)
            })),
            text_color: theme.palette().text,
            border: Border {
                color: if selected {
                    theme.palette().primary
                } else if hovered {
                    theme.extended_palette().background.strongest.text
                } else {
                    design::border(theme)
                },
                width: if selected { 2.0 } else { 1.0 },
                radius: design::radius(theme, design::Radius::Large).into(),
            },
            ..button::Style::default()
        }
    }
}

fn selected_build(state: &State) -> Option<&str> {
    state.modding_character_build.as_deref().filter(|build| {
        state
            .library_catalog
            .as_ref()
            .is_some_and(|catalog| catalog.builds.iter().any(|entry| entry == build))
    })
}

/// "Games/ModdedEvolve/EvolveGame [306e52cd]" -> ("EvolveGame", "306e52cd").
fn build_parts(build: &str) -> (&str, Option<&str>) {
    let (path, hash) = match build.rsplit_once(" [") {
        Some((path, rest)) => (path, rest.strip_suffix(']')),
        None => (build, None),
    };
    let name = path
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path);
    (name, hash)
}

fn build_picker(state: &State) -> Element<'_, Message> {
    let selected = selected_build(state);
    let Some(catalog) = &state.library_catalog else {
        return ui_kit::callout(
            design::Tone::Info,
            if state.library_startup_loading {
                "Loading your imported builds…"
            } else {
                "Import a copy of the game in the Asset Library to browse its characters here."
            },
        );
    };
    if catalog.builds.is_empty() {
        return ui_kit::callout(
            design::Tone::Info,
            "No imported builds yet. Import a copy of the game in the Asset Library first.",
        );
    }
    let mut cards = row![].spacing(10);
    for (index, build) in catalog.builds.iter().enumerate() {
        let chosen = selected == Some(build.as_str());
        let (name, hash) = build_parts(build);
        let edition = catalog
            .editions
            .get(build)
            .map_or("Game build", |edition| edition.label());
        let mut top = row![
            icons::glyph(icons::drive(), 16.0),
            text(edition).size(design::LABEL).style(design::muted),
            space().width(Fill),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if chosen {
            top = top.push(badge(BadgeKind::Enabled, "Browsing"));
        }
        let mut details = column![top, text(name).size(15)].spacing(4);
        if let Some(hash) = hash {
            details = details.push(design::caption(format!("Build {hash}")));
        }
        cards = cards.push(
            button(details)
                .padding([10, 14])
                .width(Length::Fixed(260.0))
                .style(tile_style(chosen))
                .on_press(Message::ModdingCharacterBuild(index)),
        );
    }
    column![
        row![
            design::heading("Game build"),
            design::caption("Characters, values and models are read from this imported copy. The originals are never changed."),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
        scrollable(cards).direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new().width(4).scroller_width(4),
        )),
    ]
    .spacing(8)
    .into()
}

/// Every enabled mod, so the page shows what Play with mods will load.
fn enabled_mods(state: &State) -> Element<'_, Message> {
    let manage = design::button(text("Manage mods").size(design::LABEL))
        .style(design::secondary)
        .on_press(Message::ModdingBasicAdvanced);
    let body: Element<'_, Message> = match state.modding_projects.as_deref() {
        None if state.preferences.projects.trim().is_empty() => {
            design::caption("Choose a projects folder in Settings to see your mods.").into()
        }
        None => design::caption("Checking your mods…").into(),
        Some(projects) => {
            let enabled: Vec<_> = projects.iter().filter(|project| project.enabled).collect();
            if enabled.is_empty() {
                design::caption(if projects.is_empty() {
                    "No mods yet. Edit a character's values below to make one."
                } else {
                    "No mods are enabled. The game starts unmodded."
                })
                .into()
            } else {
                let mut chips = row![].spacing(6);
                for project in enabled.iter().take(12) {
                    chips = chips.push(design::tooltip(
                        badge(BadgeKind::Enabled, project.label.as_str()),
                        project.status.as_str(),
                        None,
                        iced::widget::tooltip::Position::Bottom,
                    ));
                }
                if enabled.len() > 12 {
                    chips = chips.push(badge(
                        BadgeKind::Neutral,
                        format!("+{} more", enabled.len() - 12),
                    ));
                }
                chips.wrap().vertical_spacing(6).into()
            }
        }
    };
    let count = state
        .modding_projects
        .as_deref()
        .map(|projects| projects.iter().filter(|project| project.enabled).count());
    container(
        row![
            column![
                row![
                    icons::glyph(icons::layers(), 16.0),
                    text("Enabled mods").size(design::HEADING),
                    badge(
                        if count.unwrap_or(0) > 0 {
                            BadgeKind::Enabled
                        } else {
                            BadgeKind::Neutral
                        },
                        count.map_or("…".to_owned(), |count| count.to_string()),
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                body,
            ]
            .spacing(8)
            .width(Fill),
            manage,
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding([12, 16])
    .width(Fill)
    .style(card_style)
    .into()
}

fn character_card(state: &State, index: usize) -> Element<'_, Message> {
    let character = &CHARACTERS[index];
    let chosen = state.modding_character == Some(index);
    button(
        column![
            image(crate::handles::image(character.portrait))
                .width(Length::Fixed(CARD_WIDTH - 20.0))
                .height(Length::Fixed((CARD_WIDTH - 20.0) * 4.0 / 3.0)),
            text(character.name).size(design::BODY),
            design::caption(character.group),
        ]
        .spacing(2)
        .align_x(Alignment::Center),
    )
    .padding([10, 10])
    .width(Length::Fixed(CARD_WIDTH))
    .style(tile_style(chosen))
    .on_press(Message::ModdingCharacterSelected(index))
    .into()
}

fn wildlife_card(state: &State, index: usize) -> Element<'_, Message> {
    let (name, _) = WILDLIFE[index];
    button(
        row![
            icons::glyph(icons::tree(), 16.0),
            text(name).size(design::BODY)
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding([10, 12])
    .width(Length::Fixed(CARD_WIDTH * 1.5))
    .style(tile_style(state.modding_wildlife == Some(index)))
    .on_press(Message::ModdingWildlifeSelected(index))
    .into()
}

fn section<'a>(
    title: &'a str,
    count: usize,
    cards: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    column![
        row![
            text(title.to_uppercase())
                .size(design::CAPTION)
                .style(design::muted),
            text(count.to_string())
                .size(design::CAPTION)
                .style(design::muted),
        ]
        .spacing(8),
        iced::widget::Row::with_children(cards)
            .spacing(10)
            .wrap()
            .vertical_spacing(10),
    ]
    .spacing(8)
    .into()
}

fn roster(state: &State) -> Element<'_, Message> {
    let search = &state.modding_character_search;
    let class = state.modding_class;
    let mut sections = column![].spacing(18);
    let mut found = 0;
    for (group_index, group) in GROUPS.iter().enumerate() {
        if class != 0 && class != group_index + 1 {
            continue;
        }
        let cards: Vec<_> = CHARACTERS
            .iter()
            .enumerate()
            .filter(|(index, character)| {
                character.group == *group
                    && state.modding_available_characters.contains(index)
                    && crate::character_roster::matches_search(
                        character.name,
                        character.group,
                        search,
                    )
            })
            .map(|(index, _)| character_card(state, index))
            .collect();
        if !cards.is_empty() {
            found += cards.len();
            sections = sections.push(section(group, cards.len(), cards));
        }
    }
    if class == 0 || class == GROUPS.len() + 1 {
        let cards: Vec<_> = state
            .modding_available_wildlife
            .iter()
            .filter(|&&index| {
                crate::character_roster::matches_search(WILDLIFE[index].0, "Wildlife", search)
            })
            .map(|&index| wildlife_card(state, index))
            .collect();
        if !cards.is_empty() {
            found += cards.len();
            sections = sections.push(section("Wildlife", cards.len(), cards));
        }
    }
    if found == 0 {
        sections = sections.push(if state.modding_roster_loading {
            ui_kit::empty_state(
                None,
                "Reading this build's characters…",
                "This takes a moment the first time.",
                None,
            )
        } else if !search.is_empty() || class != 0 {
            ui_kit::empty_state(
                Some(icons::search()),
                "Nothing matches",
                "Try another name, or show every class.",
                Some((
                    "Show everyone",
                    Message::ModdingCharacterSearch(String::new()),
                )),
            )
        } else {
            ui_kit::empty_state(
                None,
                "No characters in this build",
                "Its gameplay files weren't found. Try another build.",
                None,
            )
        });
    }
    scrollable(container(sections).padding(iced::Padding {
        top: 0.0,
        right: 14.0,
        bottom: 12.0,
        left: 0.0,
    }))
    .height(Fill)
    .width(Fill)
    .into()
}

fn action<'a>(
    icon: iced::widget::svg::Handle,
    title: &'a str,
    detail: &'a str,
    primary: bool,
    message: Message,
) -> Element<'a, Message> {
    button(
        row![
            icons::glyph(icon, 18.0),
            column![
                text(title).size(design::BODY),
                text(detail).size(design::CAPTION).style(if primary {
                    |theme: &iced::Theme| iced::widget::text::Style {
                        color: Some(theme.palette().text.scale_alpha(0.8)),
                    }
                } else {
                    design::muted
                }),
            ]
            .spacing(2),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([10, 12])
    .width(Fill)
    .style(if primary {
        design::primary
    } else {
        design::secondary
    })
    .on_press(message)
    .into()
}

/// Enabled mods that change files belonging to `owns`.
fn mods_touching(state: &State, owns: impl Fn(&str) -> bool) -> Vec<&str> {
    state
        .modding_projects
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|project| project.enabled && project.modified.iter().any(|path| owns(path)))
        .map(|project| project.label.as_str())
        .collect()
}

fn details(state: &State) -> Element<'_, Message> {
    let character = state
        .modding_character
        .filter(|index| state.modding_available_characters.contains(index))
        .map(|index| &CHARACTERS[index]);
    let wildlife = state
        .modding_wildlife
        .filter(|index| state.modding_available_wildlife.contains(index))
        .map(|index| WILDLIFE[index].0);
    let content: Element<'_, Message> = match (character, wildlife) {
        (Some(character), _) => {
            let touching = mods_touching(state, |path| owns_document(character.name, path));
            column![
                container(
                    image(crate::handles::image(character.portrait))
                        .width(180)
                        .height(240)
                )
                .width(Fill)
                .center_x(Fill),
                text(character.name).size(22),
                row![badge(BadgeKind::Neutral, character.group)].spacing(6),
                action(
                    icons::sliders(),
                    "Gameplay values",
                    "Health, abilities, weapons and perks",
                    true,
                    Message::ModdingCharacterEdit(false),
                ),
                action(
                    icons::layers(),
                    "Models and skins",
                    "Find this character's models in the build",
                    false,
                    Message::ModdingCharacterEdit(true),
                ),
                touching_list(character.name, touching),
            ]
            .spacing(10)
            .into()
        }
        (None, Some(name)) => {
            let touching = mods_touching(state, |path| owns_wildlife_document(name, path));
            column![
                row![icons::glyph(icons::tree(), 22.0), text(name).size(22)]
                    .spacing(10)
                    .align_y(Alignment::Center),
                row![badge(BadgeKind::Neutral, "Wildlife")],
                action(
                    icons::sliders(),
                    "Gameplay values",
                    "Health, damage and behaviour",
                    true,
                    Message::ModdingCharacterEdit(false),
                ),
                touching_list(name, touching),
            ]
            .spacing(10)
            .into()
        }
        (None, None) => ui_kit::empty_state(
            Some(icons::cursor()),
            "Pick a character",
            "Choose anyone from the roster to change their values or find their models.",
            None,
        ),
    };
    container(scrollable(content).height(Fill))
        .padding(16)
        .width(Length::Fixed(PANEL_WIDTH))
        .height(Fill)
        .style(card_style)
        .into()
}

fn touching_list<'a>(name: &'a str, mods: Vec<&'a str>) -> Element<'a, Message> {
    let mut list = column![
        text("ENABLED MODS FOR THIS CHARACTER")
            .size(design::CAPTION)
            .style(design::muted)
    ]
    .spacing(6);
    if mods.is_empty() {
        list = list.push(design::caption(format!(
            "None. {name} plays with the build's own values."
        )));
    } else {
        for label in mods {
            list = list.push(
                row![
                    icons::glyph(icons::check(), 14.0),
                    text(label).size(design::BODY)
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
    }
    container(list).padding([8, 0]).into()
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let mut filters = vec![("All", Message::ModdingClass(0))];
    for (index, group) in GROUPS.iter().enumerate() {
        filters.push((*group, Message::ModdingClass(index + 1)));
    }
    filters.push(("Wildlife", Message::ModdingClass(GROUPS.len() + 1)));
    let mut page = column![
        row![
            container(build_picker(state)).width(Fill),
            design::button("Advanced Modding & PAKs")
                .style(design::secondary)
                .on_press(Message::ModdingBasicAdvanced),
        ]
        .spacing(12),
        enabled_mods(state),
    ]
    .spacing(14);
    if let Some(error) = state
        .modding_roster_error
        .as_ref()
        .filter(|_| selected_build(state).is_some())
    {
        page = page.push(ui_kit::callout(
            design::Tone::Danger,
            format!("Cannot read this build's characters: {error}"),
        ));
    }
    if selected_build(state).is_some() {
        page = page
            .push(
                row![
                    container(crate::forms::search_field(
                        "Find a character or class…",
                        &state.modding_character_search,
                        None,
                        Message::ModdingCharacterSearch,
                    ))
                    .width(Length::FillPortion(2)),
                    container(ui_kit::segmented(
                        &state.ui,
                        "modding-class",
                        filters,
                        state.modding_class,
                        true,
                    ))
                    .width(Length::FillPortion(3)),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .push(row![roster(state), details(state)].spacing(14).height(Fill));
    }
    page = page.push(row![
        space().width(Fill),
        design::button(text("Browse every gameplay file").size(design::LABEL))
            .style(design::text_button)
            .on_press(Message::GuidedEdit(crate::guided_modding::Action::Open(
                false
            ),)),
    ]);
    page.height(Fill).into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn build_labels_split_into_folder_and_hash() {
        assert_eq!(
            super::build_parts("Games/ModdedEvolve/EvolveGame [306e52cd]"),
            ("EvolveGame", Some("306e52cd"))
        );
        assert_eq!(
            super::build_parts("2012Evolve [683e1792]"),
            ("2012Evolve", Some("683e1792"))
        );
        assert_eq!(super::build_parts("Legacy"), ("Legacy", None));
    }
}
