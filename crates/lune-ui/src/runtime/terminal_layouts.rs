//! Shared saved-layout helpers for terminal-style surfaces.
//!
//! The tiling tree itself lives in [`super::tiling`]. This module owns the
//! higher-level saved-layout mutation rules so different terminal surfaces can
//! share consistent naming, overwrite, rename, and delete semantics.

use super::tiling::SavedAgentLayout;

/// Result of saving a layout by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveLayoutOutcome {
    Inserted { index: usize, name: String },
    Updated { index: usize, name: String },
}

/// Result of renaming a saved layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameLayoutOutcome {
    Renamed {
        index: usize,
        from: String,
        to: String,
    },
    ReplacedExisting {
        index: usize,
        from: String,
        to: String,
    },
}

/// Normalize a user-visible saved-layout name.
#[must_use]
pub fn normalize_layout_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn same_layout_name(lhs: &str, rhs: &str) -> bool {
    normalize_layout_name(lhs).eq_ignore_ascii_case(&normalize_layout_name(rhs))
}

/// Suggest the next default layout name without colliding with existing names.
#[must_use]
pub fn suggest_layout_name(layouts: &[SavedAgentLayout]) -> String {
    let mut counter = 1usize;
    loop {
        let candidate = format!("Layout {counter}");
        if layouts
            .iter()
            .all(|layout| !same_layout_name(&layout.name, &candidate))
        {
            return candidate;
        }
        counter += 1;
    }
}

/// Insert or replace a saved layout by normalized logical name.
pub fn upsert_saved_layout(
    layouts: &mut Vec<SavedAgentLayout>,
    mut saved: SavedAgentLayout,
) -> Option<SaveLayoutOutcome> {
    saved.name = normalize_layout_name(&saved.name);
    if saved.name.is_empty() {
        return None;
    }

    if let Some(index) = layouts
        .iter()
        .position(|layout| same_layout_name(&layout.name, &saved.name))
    {
        let name = saved.name.clone();
        layouts[index] = saved;
        Some(SaveLayoutOutcome::Updated { index, name })
    } else {
        let name = saved.name.clone();
        layouts.push(saved);
        Some(SaveLayoutOutcome::Inserted {
            index: layouts.len() - 1,
            name,
        })
    }
}

/// Rename a saved layout in-place. If the new name collides with another
/// layout, that layout is replaced by the renamed one and the old slot removed.
pub fn rename_saved_layout(
    layouts: &mut Vec<SavedAgentLayout>,
    index: usize,
    new_name: String,
) -> Option<RenameLayoutOutcome> {
    let normalized = normalize_layout_name(&new_name);
    if normalized.is_empty() || index >= layouts.len() {
        return None;
    }

    let from = layouts[index].name.clone();
    if same_layout_name(&from, &normalized) {
        layouts[index].name = normalized.clone();
        return Some(RenameLayoutOutcome::Renamed {
            index,
            from,
            to: normalized,
        });
    }

    let duplicate_index = layouts
        .iter()
        .enumerate()
        .find_map(|(other_index, layout)| {
            (other_index != index && same_layout_name(&layout.name, &normalized))
                .then_some(other_index)
        });

    if let Some(duplicate_index) = duplicate_index {
        let root = layouts[index].root.clone();
        layouts[duplicate_index] = SavedAgentLayout {
            name: normalized.clone(),
            root,
        };
        layouts.remove(index);
        let final_index = if index < duplicate_index {
            duplicate_index - 1
        } else {
            duplicate_index
        };
        Some(RenameLayoutOutcome::ReplacedExisting {
            index: final_index,
            from,
            to: normalized,
        })
    } else {
        layouts[index].name = normalized.clone();
        Some(RenameLayoutOutcome::Renamed {
            index,
            from,
            to: normalized,
        })
    }
}

/// Delete a saved layout by index.
pub fn delete_saved_layout(
    layouts: &mut Vec<SavedAgentLayout>,
    index: usize,
) -> Option<SavedAgentLayout> {
    (index < layouts.len()).then(|| layouts.remove(index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tiling::SavedTileNode;

    fn layout(name: &str) -> SavedAgentLayout {
        SavedAgentLayout {
            name: name.to_string(),
            root: SavedTileNode::Leaf,
        }
    }

    #[test]
    fn normalize_layout_name_trims_and_collapses_whitespace() {
        assert_eq!(normalize_layout_name("  Main   Stack  "), "Main Stack");
    }

    #[test]
    fn suggest_layout_name_skips_existing_names_case_insensitively() {
        let layouts = vec![layout("layout 1"), layout("Layout 2")];
        assert_eq!(suggest_layout_name(&layouts), "Layout 3");
    }

    #[test]
    fn upsert_saved_layout_overwrites_by_normalized_name() {
        let mut layouts = vec![layout(" Main Stack ")];
        let outcome = upsert_saved_layout(&mut layouts, layout("main   stack")).unwrap();
        assert_eq!(
            outcome,
            SaveLayoutOutcome::Updated {
                index: 0,
                name: "main stack".to_string(),
            }
        );
        assert_eq!(layouts.len(), 1);
        assert_eq!(layouts[0].name, "main stack");
    }

    #[test]
    fn rename_saved_layout_replaces_existing_duplicate_name() {
        let mut layouts = vec![layout("Alpha"), layout("Beta"), layout("Gamma")];
        let outcome = rename_saved_layout(&mut layouts, 0, " gamma ".to_string()).unwrap();
        assert_eq!(
            outcome,
            RenameLayoutOutcome::ReplacedExisting {
                index: 1,
                from: "Alpha".to_string(),
                to: "gamma".to_string(),
            }
        );
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[1].name, "gamma");
    }

    #[test]
    fn delete_saved_layout_removes_requested_entry() {
        let mut layouts = vec![layout("One"), layout("Two")];
        let deleted = delete_saved_layout(&mut layouts, 0).unwrap();
        assert_eq!(deleted.name, "One");
        assert_eq!(layouts.len(), 1);
        assert_eq!(layouts[0].name, "Two");
    }
}
