//! The split tree of one tab. Leaves are panes;
//! inner nodes split their space in two, side by side (`Row`) or stacked (`Column`).
//! Pure data, no UI, so it can be tested on its own.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Side by side: ⌘D.
    Row,
    /// Stacked: ⇧⌘D.
    Column,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node<P> {
    Leaf(P),
    Split { axis: Axis, ratio: f32, a: Box<Node<P>>, b: Box<Node<P>> },
}

impl<P: Copy + PartialEq> Node<P> {
    /// Panes in reading order: left to right, top to bottom.
    pub fn panes(&self) -> Vec<P> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<P>) {
        match self {
            Node::Leaf(p) => out.push(*p),
            Node::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    /// Splits `target` in two; the new pane goes right or below.
    pub fn split(&mut self, target: P, axis: Axis, new: P) -> bool {
        match self {
            Node::Leaf(p) if *p == target => {
                *self = Node::Split { axis, ratio: 0.5, a: Box::new(Node::Leaf(target)), b: Box::new(Node::Leaf(new)) };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, axis, new) || b.split(target, axis, new),
        }
    }

    /// Removes a pane; its sibling takes the space. Returns false if the pane
    /// is the only one (the caller closes the tab instead).
    pub fn remove(&mut self, target: P) -> bool {
        let Node::Split { a, b, .. } = self else { return false };
        let keep = if matches!(**a, Node::Leaf(p) if p == target) {
            Some((**b).clone())
        } else if matches!(**b, Node::Leaf(p) if p == target) {
            Some((**a).clone())
        } else {
            None
        };
        match keep {
            Some(sibling) => {
                *self = sibling;
                true
            }
            None => a.remove(target) || b.remove(target),
        }
    }

    /// The next (or previous) pane in reading order, wrapping around.
    pub fn cycle(&self, from: P, forward: bool) -> P {
        let panes = self.panes();
        let i = panes.iter().position(|p| *p == from).unwrap_or(0);
        let n = panes.len();
        panes[if forward { (i + 1) % n } else { (i + n - 1) % n }]
    }

    /// Sets the ratio of the split at `path` (a list of a/b turns from the root).
    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) {
        let mut node = self;
        for &go_b in path {
            match node {
                Node::Split { a, b, .. } => node = if go_b { b } else { a },
                Node::Leaf(_) => return,
            }
        }
        if let Node::Split { ratio: r, .. } = node {
            *r = ratio.clamp(0.1, 0.9);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_remove_cycle() {
        let mut t = Node::Leaf(1);
        assert!(t.split(1, Axis::Row, 2));
        assert!(t.split(2, Axis::Column, 3));
        assert_eq!(t.panes(), [1, 2, 3]);
        assert_eq!(t.cycle(3, true), 1);
        assert_eq!(t.cycle(1, false), 3);

        assert!(t.remove(2));
        assert_eq!(t.panes(), [1, 3]);
        assert!(matches!(t, Node::Split { axis: Axis::Row, .. }));
        assert!(t.remove(1));
        assert_eq!(t, Node::Leaf(3));
        assert!(!t.remove(3));
    }

    #[test]
    fn ratios_are_clamped_and_addressed_by_path() {
        let mut t = Node::Leaf(1);
        t.split(1, Axis::Row, 2);
        t.split(2, Axis::Column, 3);
        t.set_ratio(&[true], 0.99);
        let Node::Split { b, .. } = &t else { panic!() };
        assert!(matches!(**b, Node::Split { ratio, .. } if ratio == 0.9));
    }
}
