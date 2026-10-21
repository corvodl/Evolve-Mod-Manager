//! Shared Windows-style row selection. All indices are stable backing-store
//! indices; the supplied order is the currently visible, filtered row order.
use std::collections::HashSet;

#[derive(Default, Clone, Debug)]
pub struct MultiSelection {
    pub indices: HashSet<usize>,
    pub anchor: Option<usize>,
}

impl MultiSelection {
    pub fn clear(&mut self) {
        self.indices.clear();
        self.anchor = None;
    }

    pub fn click(&mut self, visible: &[usize], clicked: usize, shift: bool, ctrl: bool) {
        let Some(end) = visible.iter().position(|&index| index == clicked) else {
            return;
        };
        if shift
            && let Some(begin) = self
                .anchor
                .and_then(|anchor| visible.iter().position(|&index| index == anchor))
        {
            if !ctrl {
                self.indices.clear();
            }
            for &index in &visible[begin.min(end)..=begin.max(end)] {
                self.indices.insert(index);
            }
            return;
        }
        if ctrl {
            if !self.indices.insert(clicked) {
                self.indices.remove(&clicked);
            }
        } else {
            self.indices.clear();
            self.indices.insert(clicked);
        }
        self.anchor = Some(clicked);
    }

    pub fn primary_after(&self, clicked: usize) -> Option<usize> {
        if self.indices.contains(&clicked) {
            Some(clicked)
        } else {
            self.indices.iter().min().copied()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_select_follows_display_order_not_source_indices() {
        let mut state = MultiSelection::default();
        let order = [9, 3, 7, 1, 5];
        state.click(&order, 3, false, false);
        state.click(&order, 5, true, false);
        assert_eq!(state.indices, HashSet::from([3, 7, 1, 5]));
        assert_eq!(state.anchor, Some(3));
    }

    #[test]
    fn control_toggle_then_shift_union() {
        let mut state = MultiSelection::default();
        let order = [0, 1, 2, 3, 4, 5];
        state.click(&order, 0, false, false);
        state.click(&order, 3, false, true);
        state.click(&order, 5, true, true);
        assert_eq!(state.indices, HashSet::from([0, 3, 4, 5]));
        state.click(&order, 5, false, true);
        assert_eq!(state.indices, HashSet::from([0, 3, 4]));
    }

    #[test]
    fn missing_anchor_in_filtered_view_falls_back_to_single_click() {
        let mut state = MultiSelection::default();
        state.click(&[1, 2, 3], 1, false, false);
        state.click(&[5, 6], 6, true, false);
        assert_eq!(state.indices, HashSet::from([6]));
        assert_eq!(state.anchor, Some(6));
    }

    #[test]
    fn shift_selection_spans_paginated_filtered_catalog_indices() {
        let mut state = MultiSelection::default();
        let ordered = (0usize..260).map(|i| i * 2).collect::<Vec<_>>();
        state.click(&ordered, ordered[107], false, false);
        state.click(&ordered, ordered[139], true, false);
        assert_eq!(state.indices.len(), 33);
        assert_eq!(state.anchor, Some(ordered[107]));
        assert!(state.indices.contains(&ordered[110]));
        assert!(!state.indices.contains(&219)); // Excluded by filter.
    }

    #[test]
    fn collapsed_folders_do_not_implicitly_select_hidden_files() {
        let mut state = MultiSelection::default();
        state.click(&[1, 4], 1, false, false);
        state.click(&[1, 4], 4, true, false);
        assert_eq!(state.indices, HashSet::from([1, 4]));
    }
}
