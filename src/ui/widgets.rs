//! Small GTK4 widget-tree helpers shared across `ui/`.

use gtk4::prelude::*;

/// Walks the direct children of a GTK widget. Wraps the
/// `first_child()` / `next_sibling()` linked-list shape into an
/// `Iterator<Item = gtk4::Widget>` so consumers can use `.count()`,
/// `.find_map()`, `.filter()`, etc. instead of hand-rolled loops.
pub(crate) fn children(parent: &impl IsA<gtk4::Widget>) -> impl Iterator<Item = gtk4::Widget> {
    let mut current = parent.first_child();
    std::iter::from_fn(move || {
        let next = current.clone();
        current = current.as_ref().and_then(|w| w.next_sibling());
        next
    })
}
