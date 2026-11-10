//! Page orders of the Organize grid (pure, ported from the egui `src/organize.rs`).

use std::collections::BTreeSet;

/// New page order after moving every `selected` page one place (`delta` < 0: earlier,
/// > 0: later). Blocks move together; pages already at an end stay put.
pub fn move_selection(count: usize, selected: &[usize], delta: isize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).collect();
    let picked = |page: usize| selected.contains(&page);
    if delta < 0 {
        for index in 1..count {
            if picked(order[index]) && !picked(order[index - 1]) {
                order.swap(index - 1, index);
            }
        }
    } else if delta > 0 {
        for index in (0..count.saturating_sub(1)).rev() {
            if picked(order[index]) && !picked(order[index + 1]) {
                order.swap(index, index + 1);
            }
        }
    }
    order
}

/// Where the pages of `moved` are in `order` (a permutation), in order.
pub fn new_places(order: &[usize], moved: &[usize]) -> Vec<usize> {
    order
        .iter()
        .enumerate()
        .filter(|(_, page)| moved.contains(page))
        .map(|(index, _)| index)
        .collect()
}

/// Move the dragged page (or the whole selection containing it) before or `after` `target`.
/// Returns the new order and the new indices of the moved pages; `None` if nothing changes.
pub fn reorder(
    count: usize,
    selection: &BTreeSet<usize>,
    dragged: usize,
    target: usize,
    after: bool,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let moved: Vec<usize> = if selection.contains(&dragged) {
        selection.iter().copied().collect()
    } else {
        vec![dragged]
    };
    if moved.contains(&target) {
        return None;
    }
    let mut order: Vec<usize> = (0..count).filter(|page| !moved.contains(page)).collect();
    let at = order.iter().position(|&page| page == target)? + usize::from(after);
    order.splice(at..at, moved.iter().copied());
    if order.iter().enumerate().all(|(index, &page)| index == page) {
        return None;
    }
    Some((order, (at..at + moved.len()).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_single_page() {
        assert_eq!(move_selection(4, &[2], -1), vec![0, 2, 1, 3]);
        assert_eq!(move_selection(4, &[2], 1), vec![0, 1, 3, 2]);
    }

    #[test]
    fn moves_block_together() {
        assert_eq!(move_selection(4, &[1, 2], -1), vec![1, 2, 0, 3]);
        assert_eq!(move_selection(4, &[1, 2], 1), vec![0, 3, 1, 2]);
    }

    #[test]
    fn moves_scattered_pages() {
        assert_eq!(move_selection(5, &[1, 3], -1), vec![1, 0, 3, 2, 4]);
        assert_eq!(move_selection(5, &[1, 3], 1), vec![0, 2, 1, 4, 3]);
        assert_eq!(new_places(&[1, 0, 3, 2, 4], &[1, 3]), vec![0, 2]);
    }

    #[test]
    fn clamps_at_ends() {
        assert_eq!(move_selection(3, &[0], -1), vec![0, 1, 2]);
        assert_eq!(move_selection(3, &[0, 2], -1), vec![0, 2, 1]);
        assert_eq!(move_selection(3, &[2], 1), vec![0, 1, 2]);
        assert_eq!(move_selection(0, &[], 1), Vec::<usize>::new());
    }

    #[test]
    fn dropping_moves_the_selection_next_to_the_target() {
        let selected: BTreeSet<usize> = [0, 1].into();
        assert_eq!(
            reorder(5, &selected, 1, 3, true),
            Some((vec![2, 3, 0, 1, 4], vec![2, 3]))
        );
        // A page outside the selection moves alone.
        assert_eq!(
            reorder(5, &selected, 4, 0, false),
            Some((vec![4, 0, 1, 2, 3], vec![0]))
        );
        assert_eq!(reorder(5, &selected, 0, 1, false), None);
        assert_eq!(reorder(5, &BTreeSet::new(), 2, 1, true), None);
    }
}
