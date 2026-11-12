//! Side-by-side line diff (original ↔ changed) for text assets, XML and Lua
//! mods. Uses the `similar` crate; long unchanged stretches collapse to a
//! single "N unchanged lines" row so the changes stay in view.

use std::time::Duration;

use iced::widget::{column, container, row, scrollable, text};
use iced::{Background, Element, Fill, Font, Length, Theme};
use similar::{DiffTag, TextDiff};

use crate::Message;
use crate::design::{self, Tone};

/// Unchanged lines kept around each change.
const CONTEXT: usize = 3;
/// Rows rendered at most; very large diffs show a notice instead.
const MAX_ROWS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Same,
    Removed,
    Added,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub number: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Pair {
        kind: Kind,
        left: Option<Line>,
        right: Option<Line>,
    },
    Collapsed(usize),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub added: usize,
    pub removed: usize,
}

fn line(number: usize, value: &str) -> Line {
    Line {
        number: number + 1,
        text: value.trim_end_matches(['\n', '\r']).to_owned(),
    }
}

/// Compute side-by-side rows with collapsed unchanged regions.
pub fn compute(old: &str, new: &str) -> (Vec<Row>, Summary) {
    let diff = TextDiff::configure()
        .timeout(Duration::from_millis(250))
        .diff_lines(old, new);
    let old_lines: Vec<&str> = diff.old_slices().to_vec();
    let new_lines: Vec<&str> = diff.new_slices().to_vec();
    let mut rows = Vec::new();
    let mut summary = Summary::default();
    let groups = diff.grouped_ops(CONTEXT);
    let mut cursor = 0usize; // next old line not yet shown
    for group in &groups {
        let Some(first) = group.first() else { continue };
        let start = first.old_range().start;
        if start > cursor {
            rows.push(Row::Collapsed(start - cursor));
        }
        for op in group {
            let (tag, old_range, new_range) = op.as_tag_tuple();
            match tag {
                DiffTag::Equal => {
                    for (o, n) in old_range.clone().zip(new_range.clone()) {
                        rows.push(Row::Pair {
                            kind: Kind::Same,
                            left: Some(line(o, old_lines[o])),
                            right: Some(line(n, new_lines[n])),
                        });
                    }
                }
                DiffTag::Delete => {
                    summary.removed += old_range.len();
                    for o in old_range.clone() {
                        rows.push(Row::Pair {
                            kind: Kind::Removed,
                            left: Some(line(o, old_lines[o])),
                            right: None,
                        });
                    }
                }
                DiffTag::Insert => {
                    summary.added += new_range.len();
                    for n in new_range.clone() {
                        rows.push(Row::Pair {
                            kind: Kind::Added,
                            left: None,
                            right: Some(line(n, new_lines[n])),
                        });
                    }
                }
                DiffTag::Replace => {
                    summary.removed += old_range.len();
                    summary.added += new_range.len();
                    let count = old_range.len().max(new_range.len());
                    for index in 0..count {
                        let o = old_range.start + index;
                        let n = new_range.start + index;
                        rows.push(Row::Pair {
                            kind: Kind::Changed,
                            left: old_range.contains(&o).then(|| line(o, old_lines[o])),
                            right: new_range.contains(&n).then(|| line(n, new_lines[n])),
                        });
                    }
                }
            }
            cursor = old_range.end;
        }
    }
    if !groups.is_empty() && cursor < old_lines.len() {
        rows.push(Row::Collapsed(old_lines.len() - cursor));
    }
    (rows, summary)
}

fn cell<'a>(value: Option<Line>, tone: Option<Tone>) -> Element<'a, Message> {
    let (number, body) = match value {
        Some(line) => (line.number.to_string(), line.text),
        None => (String::new(), String::new()),
    };
    container(
        row![
            text(number)
                .size(11)
                .font(Font::MONOSPACE)
                .width(Length::Fixed(44.0))
                .style(design::muted),
            text(body).size(12).font(Font::MONOSPACE).width(Fill),
        ]
        .spacing(8),
    )
    .padding([1, 6])
    .width(Length::FillPortion(1))
    .style(move |theme: &Theme| container::Style {
        background: tone.map(|tone| {
            Background::Color(design::with_alpha(design::tone_color(theme, tone), 0.16))
        }),
        ..container::Style::default()
    })
    .into()
}

/// Render the diff of `old` → `new` with a summary header.
pub fn view<'a>(old: &str, new: &str, left_title: &'a str, right_title: &'a str) -> Element<'a, Message> {
    let (rows, summary) = compute(old, new);
    if rows.is_empty() {
        return crate::ui_kit::empty_state(
            None,
            "No changes",
            "The changed file is identical to the original.",
            None,
        );
    }
    let header = row![
        text(left_title).size(12).width(Length::FillPortion(1)),
        text(right_title).size(12).width(Length::FillPortion(1)),
    ]
    .spacing(2);
    let stats = row![
        text(format!("+{}", summary.added))
            .size(12)
            .style(|theme: &Theme| text::Style {
                color: Some(theme.palette().success)
            }),
        text(format!("−{}", summary.removed))
            .size(12)
            .style(|theme: &Theme| text::Style {
                color: Some(theme.palette().danger)
            }),
    ]
    .spacing(10);
    let truncated = rows.len() > MAX_ROWS;
    let mut list = column![].spacing(0);
    for entry in rows.into_iter().take(MAX_ROWS) {
        let element: Element<'a, Message> = match entry {
            Row::Collapsed(count) => container(
                text(format!("⋯ {count} unchanged line{}", if count == 1 { "" } else { "s" }))
                    .size(11)
                    .style(design::muted),
            )
            .padding([4, 10])
            .width(Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(design::field(theme))),
                ..container::Style::default()
            })
            .into(),
            Row::Pair { kind, left, right } => {
                let (left_tone, right_tone) = match kind {
                    Kind::Same => (None, None),
                    Kind::Removed => (Some(Tone::Danger), None),
                    Kind::Added => (None, Some(Tone::Success)),
                    Kind::Changed => (
                        left.as_ref().map(|_| Tone::Danger),
                        right.as_ref().map(|_| Tone::Success),
                    ),
                };
                row![cell(left, left_tone), cell(right, right_tone)]
                    .spacing(2)
                    .into()
            }
        };
        list = list.push(element);
    }
    let mut content = column![row![header, stats].spacing(12), scrollable(list).height(Fill)]
        .spacing(8)
        .height(Fill);
    if truncated {
        content = content.push(crate::ui_kit::callout(
            Tone::Warning,
            format!("Showing the first {MAX_ROWS} rows of this diff."),
        ));
    }
    content.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_has_no_rows() {
        let (rows, summary) = compute("a\nb\n", "a\nb\n");
        assert!(rows.is_empty());
        assert_eq!(summary, Summary::default());
    }

    #[test]
    fn changes_are_paired_and_long_unchanged_runs_collapse() {
        let old: String = (1..=40).map(|n| format!("line {n}\n")).collect();
        let new = old.replace("line 20\n", "line twenty\n") + "extra\n";
        let (rows, summary) = compute(&old, &new);
        assert_eq!(summary, Summary { added: 2, removed: 1 });
        assert!(matches!(rows[0], Row::Collapsed(16)));
        let changed = rows
            .iter()
            .find(|row| matches!(row, Row::Pair { kind: Kind::Changed, .. }))
            .unwrap();
        if let Row::Pair { left, right, .. } = changed {
            assert_eq!(left.as_ref().unwrap().text, "line 20");
            assert_eq!(left.as_ref().unwrap().number, 20);
            assert_eq!(right.as_ref().unwrap().text, "line twenty");
        }
        // Collapsed middle section between the two hunks.
        assert!(rows.iter().filter(|row| matches!(row, Row::Collapsed(_))).count() >= 2);
        assert!(matches!(
            rows.last(),
            Some(Row::Pair { kind: Kind::Added, .. })
        ));
    }
}
