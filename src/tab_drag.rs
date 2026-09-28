//! Where every chip of the tab strip sits while one of them is being dragged.
//!
//! The dragged chip follows the pointer sideways and never leaves the strip;
//! the others slide to open the slot it would drop into. Everything here is
//! arithmetic over the chips' bounds as they were when the drag began, so the
//! result does not depend on where the chips have since been drawn.

use std::{cell::RefCell, rc::Rc};

use gpui::{Bounds, Pixels};

use crate::{scroller::Shift, session::TabKey};

/// The value a chip's drag carries. Which chip it is lives in the workspace's
/// `TabStrip`; gpui only needs a type to tell this drag from any other.
pub(crate) struct DragTab;

/// A drag in progress: the chips as they sat when it began, and where the
/// pointer has got to since.
pub(crate) struct Drag {
    pub(crate) key: TabKey,
    pub(crate) slots: Vec<Slot>,
    pub(crate) index: usize,
    pub(crate) grab: f32,
    pub(crate) pointer_x: f32,
}

impl Drag {
    pub(crate) fn layout(&self) -> Layout {
        layout(&self.slots, self.index, self.grab, self.pointer_x)
    }
}

pub(crate) type ChipBounds = Rc<RefCell<Vec<(TabKey, Bounds<Pixels>)>>>;

/// Every chip's bounds as last drawn, refreshed by the chips themselves, and
/// the drag over them if there is one.
#[derive(Default)]
pub(crate) struct TabStrip {
    pub(crate) drag: Option<Drag>,
    pub(crate) bounds: ChipBounds,
    /// Where each chip is drawn, gliding to where the drag has made room.
    pub(crate) shift: Shift<TabKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Slot {
    pub(crate) key: TabKey,
    pub(crate) left: f32,
    pub(crate) width: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    /// How far each chip is drawn from where it started, in the slots' order.
    pub(crate) offsets: Vec<f32>,
    /// Where the dragged chip would land among the others.
    pub(crate) target: usize,
    /// Where each chip would sit, as an offset from where it started, if the
    /// drag ended now: the dragged chip's own slot, not where the pointer has it.
    pub(crate) landing: Vec<f32>,
}

pub(crate) fn layout(slots: &[Slot], dragged: usize, grab: f32, pointer_x: f32) -> Layout {
    let Some(first) = slots.first() else {
        return Layout {
            offsets: Vec::new(),
            target: 0,
            landing: Vec::new(),
        };
    };
    let last = &slots[slots.len() - 1];
    let gap = match slots.get(1) {
        Some(second) => (second.left - (first.left + first.width)).max(0.),
        None => 0.,
    };
    let width = slots[dragged].width;
    let desired =
        (pointer_x - grab).clamp(first.left, (last.left + last.width - width).max(first.left));
    // The leading edge decides: the chip has to get its edge past a
    // neighbour's middle to take its place, which is also what lets it reach
    // either end of a strip it cannot leave.
    let before = slots[..dragged]
        .iter()
        .filter(|slot| desired >= slot.left + slot.width / 2.)
        .count();
    let passed = slots[dragged + 1..]
        .iter()
        .filter(|slot| desired + width > slot.left + slot.width / 2.)
        .count();
    let target = before + passed;

    let mut order: Vec<usize> = (0..slots.len()).filter(|index| *index != dragged).collect();
    order.insert(target, dragged);
    let mut offsets = vec![0.; slots.len()];
    let mut landing = vec![0.; slots.len()];
    let mut cursor = first.left;
    for index in order {
        landing[index] = cursor - slots[index].left;
        offsets[index] = match index == dragged {
            true => desired - slots[index].left,
            false => landing[index],
        };
        cursor += slots[index].width + gap;
    }
    Layout {
        offsets,
        target,
        landing,
    }
}

/// `keys` with `dragged` taken out and put back at `target`.
pub(crate) fn reordered(keys: &[TabKey], dragged: &TabKey, target: usize) -> Vec<TabKey> {
    let mut order: Vec<TabKey> = keys.iter().filter(|key| *key != dragged).cloned().collect();
    order.insert(target.min(order.len()), dragged.clone());
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots() -> Vec<Slot> {
        [(0., 100.), (104., 60.), (168., 80.)]
            .into_iter()
            .enumerate()
            .map(|(index, (left, width))| Slot {
                key: TabKey::Object(index as u64),
                left,
                width,
            })
            .collect()
    }

    #[test]
    fn a_chip_that_has_not_moved_leaves_everything_where_it_is() {
        let result = layout(&slots(), 1, 10., 114.);
        assert_eq!(result.target, 1);
        assert_eq!(result.offsets, vec![0., 0., 0.]);
    }

    #[test]
    fn dragging_past_a_neighbours_middle_swaps_the_two() {
        // The second chip, grabbed at its left edge, dragged over the third's
        // middle: the third slides into the second's slot, the second lands
        // after it.
        let result = layout(&slots(), 1, 0., 180.);
        assert_eq!(result.target, 2);
        assert_eq!(result.offsets[2], -64.);
        assert_eq!(result.offsets[0], 0.);
        assert_eq!(result.offsets[1], 180. - 104.);
    }

    #[test]
    fn the_dragged_chip_stays_inside_the_strip() {
        let result = layout(&slots(), 0, 0., -500.);
        assert_eq!(result.offsets[0], 0.);
        assert_eq!(result.target, 0);
        let result = layout(&slots(), 0, 0., 900.);
        assert_eq!(result.target, 2);
        assert_eq!(result.offsets[0], 248. - 100.);
    }

    #[test]
    fn a_dropped_chip_lands_at_the_target() {
        let keys = [TabKey::Object(0), TabKey::Object(1), TabKey::Object(2)];
        assert_eq!(
            reordered(&keys, &TabKey::Object(0), 2),
            [TabKey::Object(1), TabKey::Object(2), TabKey::Object(0)]
        );
    }
}
