//! The menus of the macOS menu bar, as Outlook for Mac has them: Bestand, the module's own
//! menu (Bericht, Agenda, …), Map, Weergeven and Help, next to the Apple menu, instead of the
//! ribbon's tabs in the window.
//!
//! The menus are not a second list of commands: each frame the ribbon tabs run once more in
//! "capture" mode, where every ribbon button notes its label, whether it can be used and
//! whether it is on, instead of drawing itself. A click in the menu bar is handed back the next
//! frame to that same capture run, where the button with that label reports a click and runs
//! the code it runs in the ribbon. So the menus always hold what the ribbon holds.

use std::cell::RefCell;

/// One line of a menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// A command; `checked` is set for one that can be on or off.
    Item {
        label: String,
        enabled: bool,
        checked: Option<bool>,
    },
    Separator,
    /// A choice of one of several, such as the sort order.
    Submenu {
        label: String,
        entries: Vec<Entry>,
    },
}

/// A menu of the menu bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub title: String,
    pub entries: Vec<Entry>,
}

impl Menu {
    /// Whether `other` has the same lines, so only the states of its items may differ.
    pub fn same_shape(&self, other: &Menu) -> bool {
        fn shape(e: &[Entry]) -> Vec<(u8, &str, bool)> {
            e.iter()
                .flat_map(|e| match e {
                    Entry::Item { label, checked, .. } => vec![(0, label.as_str(), checked.is_some())],
                    Entry::Separator => vec![(1, "", false)],
                    Entry::Submenu { label, entries } => {
                        let mut v = vec![(2, label.as_str(), false)];
                        v.extend(shape(entries));
                        v.push((3, "", false));
                        v
                    }
                })
                .collect()
        }
        self.title == other.title && shape(&self.entries) == shape(&other.entries)
    }
}

/// Where a click landed: the labels from the menu down to the item.
pub type Path = Vec<String>;

struct Capture {
    /// The lines noted so far; a submenu being noted is the last of `open`.
    entries: Vec<Entry>,
    open: Vec<(String, Vec<Entry>)>,
    click: Option<Path>,
}

thread_local! {
    static CAPTURE: RefCell<Option<Capture>> = const { RefCell::new(None) };
}

/// Whether the ribbon is being noted for the menu bar rather than drawn.
pub(crate) fn capturing() -> bool {
    CAPTURE.with(|c| c.borrow().is_some())
}

/// Runs `f` in capture mode; gives the lines it noted, with doubled or stray separators left out.
pub(crate) fn capture(click: Option<Path>, f: impl FnOnce()) -> Vec<Entry> {
    CAPTURE.with(|c| *c.borrow_mut() = Some(Capture { entries: Vec::new(), open: Vec::new(), click }));
    f();
    let entries = CAPTURE.with(|c| c.borrow_mut().take()).map(|c| c.entries).unwrap_or_default();
    tidy(entries)
}

fn tidy(entries: Vec<Entry>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for e in entries {
        if e == Entry::Separator && matches!(out.last(), None | Some(Entry::Separator)) {
            continue;
        }
        out.push(e);
    }
    if out.last() == Some(&Entry::Separator) {
        out.pop();
    }
    out
}

/// Notes a command; true when the menu bar's click was on it (and it can be used).
pub(crate) fn item(label: &str, enabled: bool, checked: Option<bool>) -> bool {
    CAPTURE.with(|c| {
        let mut c = c.borrow_mut();
        let Some(c) = c.as_mut() else { return false };
        let mut path: Path = c.open.iter().map(|(l, _)| l.clone()).collect();
        path.push(label.to_string());
        let entry = Entry::Item { label: label.to_string(), enabled, checked };
        match c.open.last_mut() {
            Some((_, entries)) => entries.push(entry),
            None => c.entries.push(entry),
        }
        enabled && c.click.as_ref() == Some(&path)
    })
}

/// Notes a line between groups of commands.
pub(crate) fn separator() {
    CAPTURE.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            match c.open.last_mut() {
                Some((_, entries)) => entries.push(Entry::Separator),
                None => c.entries.push(Entry::Separator),
            }
        }
    });
}

/// Notes a choice of one of `options` (each with whether it is the current one) as a submenu
/// named `label`; gives the index of the one clicked.
pub(crate) fn choice(label: &str, options: &[(String, bool)]) -> Option<usize> {
    submenu(label, || options.iter().position(|(name, on)| item(name, true, Some(*on))))
}

/// Notes what `f` notes as a submenu named `label`.
pub(crate) fn submenu<R>(label: &str, f: impl FnOnce() -> R) -> R {
    CAPTURE.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() {
            c.open.push((label.to_string(), Vec::new()));
        }
    });
    let r = f();
    CAPTURE.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut()
            && let Some((label, entries)) = c.open.pop()
        {
            let entry = Entry::Submenu { label, entries: tidy(entries) };
            match c.open.last_mut() {
                Some((_, e)) => e.push(entry),
                None => c.entries.push(entry),
            }
        }
    });
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_notes_the_items_and_reports_the_clicked_one() {
        let mut clicked = Vec::new();
        let entries = capture(Some(vec!["Sorteren".into(), "Van".into()]), || {
            separator();
            if item("Nieuw", true, None) {
                clicked.push("Nieuw");
            }
            separator();
            separator();
            if item("Verwijderen", false, None) {
                clicked.push("Verwijderen");
            }
            if let Some(i) = choice("Sorteren", &[("Datum".into(), true), ("Van".into(), false)]) {
                clicked.push(["Datum", "Van"][i]);
            }
            separator();
        });
        assert_eq!(clicked, ["Van"]);
        assert_eq!(entries.len(), 4, "no separator at the ends or twice in a row: {entries:?}");
        assert!(matches!(&entries[3], Entry::Submenu { label, entries } if label == "Sorteren" && entries.len() == 2));
        assert!(!capturing());
    }

    #[test]
    fn a_disabled_item_is_never_clicked() {
        let mut ran = false;
        capture(Some(vec!["Verwijderen".into()]), || ran = item("Verwijderen", false, None));
        assert!(!ran);
    }
}
