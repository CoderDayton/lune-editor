//! Shared saved-layout helpers for terminal-style surfaces.
//!
//! The tiling tree itself lives in [`super::tiling`]. This module owns the
//! higher-level saved-layout mutation rules so different terminal surfaces can
//! share consistent naming, overwrite, rename, and delete semantics.

use super::tiling::{PRESET_LIST, SavedAgentLayout};

/// Maximum characters allowed in a saved layout name after trimming.
pub const MAX_LAYOUT_NAME_LEN: usize = 48;

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

/// Validate a user-typed saved-layout name.
///
/// Returns `None` when the name is acceptable, or a short human-readable
/// error message otherwise. Rules:
///
/// - Must not be empty after trimming/collapsing whitespace.
/// - Must not exceed [`MAX_LAYOUT_NAME_LEN`] characters.
/// - Must not collide with a preset layout name (case-insensitive).
#[must_use]
pub fn validate_layout_name(name: &str) -> Option<&'static str> {
    let normalized = normalize_layout_name(name);
    if normalized.is_empty() {
        return Some("Layout name cannot be empty");
    }
    if normalized.chars().count() > MAX_LAYOUT_NAME_LEN {
        return Some("Layout name is too long");
    }
    if PRESET_LIST
        .iter()
        .any(|preset| preset.name.eq_ignore_ascii_case(&normalized))
    {
        return Some("Name clashes with a built-in preset");
    }
    None
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

    let name = saved.name.clone();

    if let Some(index) = layouts
        .iter()
        .position(|layout| same_layout_name(&layout.name, &saved.name))
    {
        layouts[index] = saved;
        Some(SaveLayoutOutcome::Updated { index, name })
    } else {
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
    new_name: &str,
) -> Option<RenameLayoutOutcome> {
    let normalized = normalize_layout_name(new_name);
    if normalized.is_empty() || index >= layouts.len() {
        return None;
    }

    let from = layouts[index].name.clone();
    if same_layout_name(&from, &normalized) {
        layouts[index].name.clone_from(&normalized);
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
        let pane_kinds = layouts[index].pane_kinds.clone();
        layouts[duplicate_index] = SavedAgentLayout {
            name: normalized.clone(),
            root,
            pane_kinds,
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
        layouts[index].name.clone_from(&normalized);
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

/// Swap the layout at `index` with its neighbour in the given direction.
///
/// `delta` is `-1` to move up and `1` to move down. Returns the new index
/// when the swap succeeded, or `None` when the move would go out of bounds.
pub fn reorder_saved_layout(
    layouts: &mut [SavedAgentLayout],
    index: usize,
    delta: isize,
) -> Option<usize> {
    if index >= layouts.len() {
        return None;
    }
    let target = index.checked_add_signed(delta)?;
    if target >= layouts.len() {
        return None;
    }
    layouts.swap(index, target);
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tiling::SavedTileNode;

    fn layout(name: &str) -> SavedAgentLayout {
        SavedAgentLayout {
            name: name.to_string(),
            root: SavedTileNode::Leaf,
            pane_kinds: Vec::new(),
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
        let outcome = rename_saved_layout(&mut layouts, 0, " gamma ").unwrap();
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
    fn validate_layout_name_accepts_sensible_names() {
        assert!(validate_layout_name("Main Stack").is_none());
        assert!(validate_layout_name("  padded name ").is_none());
    }

    #[test]
    fn validate_layout_name_rejects_empty_and_whitespace() {
        assert!(validate_layout_name("").is_some());
        assert!(validate_layout_name("   ").is_some());
    }

    #[test]
    fn validate_layout_name_rejects_preset_collisions() {
        for preset in PRESET_LIST {
            assert!(
                validate_layout_name(preset.name).is_some(),
                "expected preset name {:?} to be rejected",
                preset.name
            );
            assert!(
                validate_layout_name(&preset.name.to_lowercase()).is_some(),
                "expected lowercased preset {:?} to be rejected",
                preset.name
            );
        }
    }

    #[test]
    fn validate_layout_name_rejects_too_long() {
        let long = "x".repeat(MAX_LAYOUT_NAME_LEN + 1);
        assert!(validate_layout_name(&long).is_some());
    }

    #[test]
    fn reorder_saved_layout_swaps_with_neighbor() {
        let mut layouts = vec![layout("A"), layout("B"), layout("C")];
        assert_eq!(reorder_saved_layout(&mut layouts, 0, 1), Some(1));
        assert_eq!(layouts[0].name, "B");
        assert_eq!(layouts[1].name, "A");

        assert_eq!(reorder_saved_layout(&mut layouts, 2, -1), Some(1));
        assert_eq!(layouts[1].name, "C");
        assert_eq!(layouts[2].name, "A");
    }

    #[test]
    fn reorder_saved_layout_rejects_out_of_bounds() {
        let mut layouts = vec![layout("A"), layout("B")];
        assert_eq!(reorder_saved_layout(&mut layouts, 0, -1), None);
        assert_eq!(reorder_saved_layout(&mut layouts, 1, 1), None);
        assert_eq!(reorder_saved_layout(&mut layouts, 5, 1), None);
        assert_eq!(layouts[0].name, "A");
        assert_eq!(layouts[1].name, "B");
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
