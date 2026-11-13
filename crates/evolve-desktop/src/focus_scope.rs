//! Tab and Shift+Tab focus traversal, scoped to one window.
//!
//! iced's `focus_next` walks every open window as one list, so Tab in the
//! main window could land in the Level Editor. Each window's root here is a
//! container with its own id, and an open dialog or palette inside it traps
//! focus.

use iced::Rectangle;
use iced::Task;
use iced::advanced::widget::operation::{self, Focusable, Operation, Outcome};
use iced::advanced::widget::{Id, operate};
use iced::window;

pub const DIALOG: Id = Id::new("focus-scope-dialog");
pub const PALETTE: Id = Id::new("focus-scope-palette");

pub fn window(id: window::Id) -> Id {
    Id::from(format!("focus-scope-window-{id:?}"))
}

/// Moves focus to the next (or previous) focusable widget in `window_id`,
/// staying inside the palette or a dialog when one is open there.
pub fn cycle<T: Send + 'static>(window_id: window::Id, backwards: bool) -> Task<T> {
    // Later scopes win when present: the palette draws above a dialog.
    let scopes = vec![window(window_id), DIALOG, PALETTE];
    operate(operation::then(Count::new(scopes, backwards), Move::new)).discard()
}

#[derive(Debug, Clone, Copy, Default)]
struct ScopeCount {
    seen: bool,
    total: usize,
    focused: Option<usize>,
}

/// Which scopes enclose the widget being visited. Scope 0 is the window;
/// the others only count inside it.
#[derive(Debug, Clone)]
struct Walk {
    scopes: Vec<Id>,
    pending: Option<usize>,
    open: Vec<usize>,
}

impl Walk {
    fn new(scopes: Vec<Id>) -> Self {
        Self {
            scopes,
            pending: None,
            open: Vec::new(),
        }
    }

    fn container(&mut self, id: Option<&Id>) {
        self.pending = id.and_then(|id| self.scopes.iter().position(|scope| scope == id));
    }

    fn in_window(&self) -> bool {
        self.open.contains(&0)
    }

    /// Containers call `container` then `traverse` for the same widget, so
    /// a scope matched in `container` encloses what `operate` visits.
    fn enter(&mut self) -> Option<usize> {
        let entering = self
            .pending
            .take()
            .filter(|&scope| scope == 0 || self.in_window());
        if let Some(scope) = entering {
            self.open.push(scope);
        }
        entering
    }

    fn leave(&mut self, entered: Option<usize>) {
        if entered.is_some() {
            self.open.pop();
        }
    }
}

#[derive(Debug, Clone)]
struct Plan {
    scopes: Vec<Id>,
    counts: Vec<ScopeCount>,
    backwards: bool,
}

struct Count {
    walk: Walk,
    plan: Plan,
}

impl Count {
    fn new(scopes: Vec<Id>, backwards: bool) -> Self {
        let counts = vec![ScopeCount::default(); scopes.len()];
        Self {
            walk: Walk::new(scopes.clone()),
            plan: Plan {
                scopes,
                counts,
                backwards,
            },
        }
    }
}

impl Operation<Plan> for Count {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Plan>)) {
        let entered = self.walk.enter();
        if let Some(scope) = entered {
            self.plan.counts[scope].seen = true;
        }
        operate(self);
        self.walk.leave(entered);
    }

    fn container(&mut self, id: Option<&Id>, _bounds: Rectangle) {
        self.walk.container(id);
    }

    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        for &scope in &self.walk.open {
            let count = &mut self.plan.counts[scope];
            if state.is_focused() {
                count.focused = Some(count.total);
            }
            count.total += 1;
        }
    }

    fn finish(&self) -> Outcome<Plan> {
        Outcome::Some(self.plan.clone())
    }
}

struct Move {
    walk: Walk,
    scope: Option<usize>,
    destination: Option<usize>,
    current: usize,
}

impl Move {
    fn new(plan: Plan) -> Self {
        let scope = (0..plan.counts.len())
            .rev()
            .find(|&index| plan.counts[index].seen);
        let destination = scope.and_then(|index| step(plan.counts[index], plan.backwards));
        Self {
            walk: Walk::new(plan.scopes),
            scope,
            destination,
            current: 0,
        }
    }
}

fn step(count: ScopeCount, backwards: bool) -> Option<usize> {
    let last = count.total.checked_sub(1)?;
    Some(match (count.focused, backwards) {
        (None, false) => 0,
        (None, true) => last,
        (Some(at), false) if at == last => 0,
        (Some(at), false) => at + 1,
        (Some(0), true) => last,
        (Some(at), true) => at - 1,
    })
}

impl Operation for Move {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        let entered = self.walk.enter();
        operate(self);
        self.walk.leave(entered);
    }

    fn container(&mut self, id: Option<&Id>, _bounds: Rectangle) {
        self.walk.container(id);
    }

    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if !self.walk.in_window() {
            return;
        }
        let in_scope = self
            .scope
            .is_some_and(|scope| self.walk.open.contains(&scope));
        if in_scope && Some(self.current) == self.destination {
            state.focus();
        } else if state.is_focused() {
            state.unfocus();
        }
        if in_scope {
            self.current += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(total: usize, focused: Option<usize>) -> ScopeCount {
        ScopeCount {
            seen: true,
            total,
            focused,
        }
    }

    #[test]
    fn tab_wraps_both_ways_and_starts_at_an_edge() {
        assert_eq!(step(count(3, None), false), Some(0));
        assert_eq!(step(count(3, None), true), Some(2));
        assert_eq!(step(count(3, Some(1)), false), Some(2));
        assert_eq!(step(count(3, Some(2)), false), Some(0));
        assert_eq!(step(count(3, Some(0)), true), Some(2));
        assert_eq!(step(count(0, None), false), None);
    }
}
