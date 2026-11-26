//! The disk organizer's map: the folder shown laid out as a squarified treemap (Bruls, Huizing and van Wijk, 2000),
//! two levels at a time. Its contents are the first level, rounded blocks with gaps between them; each folder among
//! them large enough is a quiet ground with its name above what it holds, the second level, a narrow margin in from its
//! sides and foot (the ground's own edges in line with the blocks beside it). A folder drawn as one
//! block is drawn as what it holds all the way down, pieces split by one-DIP lines (see `pieces`). Deeper only by
//! going in.

use super::{Held, Kind, Rank, Tree, What};

/// One corner radius for every block (Windows 11's for small controls), the gaps between blocks at the first level
/// and the second, a first-level folder's name line and the margin of what it holds in from its sides and foot.
pub const RADIUS: f32 = 4.0;
pub const GAP_FIRST: f32 = 6.0;
pub const GAP_SECOND: f32 = 3.0;
pub const LABEL: f32 = 26.0;
pub const INSET: f32 = 4.0;
/// The least a block on the page is either way: about what a pointer hits surely.
pub const SIDE_LEAST: f32 = 12.0;
/// The least a folder's piece is either way to be split into what it holds, and how deep pieces go at most.
pub const PIECE_LEAST: f32 = 5.0;
pub const PIECES_DEEPEST: usize = 8;
/// How large a first-level folder must be drawn to show what it holds as a second level.
pub const GROUP_WIDTH: f32 = 56.0;
pub const GROUP_HEIGHT: f32 = LABEL + 24.0;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// What a block or a piece is: a node of the tree (see `Tree`), or several of a folder's (by its node) too small to
/// draw where they land, together.
#[derive(Clone, PartialEq, Debug)]
pub enum Item {
    Node(usize),
    Many { folder: usize, members: Vec<usize> },
}

impl Item {
    pub fn weight(&self, tree: &Tree) -> u64 {
        match self {
            Item::Node(node) => tree.nodes[*node].held.bytes,
            Item::Many { members, .. } => members.iter().map(|&member| tree.nodes[member].held.bytes).sum(),
        }
    }

    /// What it holds, all of it together.
    pub fn held(&self, tree: &Tree) -> Held {
        match self {
            Item::Node(node) => tree.nodes[*node].held,
            Item::Many { members, .. } => {
                let mut held = Held::default();
                for &member in members {
                    let other = &tree.nodes[member].held;
                    held.bytes += other.bytes;
                    held.files += other.files;
                    held.written = held.written.max(other.written);
                    held.guarded |= other.guarded;
                    for (mine, theirs) in held.kinds.iter_mut().zip(other.kinds) {
                        *mine += theirs;
                    }
                }
                held
            }
        }
    }

    pub fn kind(&self, tree: &Tree) -> Kind {
        self.held(tree).kind()
    }

    /// Several together's: where they start in their folder's order (see `Rank`), past the smallest of those beside
    /// them drawn on their own; none where none is (they are all the folder holds).
    pub fn bound(&self, tree: &Tree) -> Option<Rank> {
        let Item::Many { folder, members } = self else { return None };
        let node = |at: usize| &tree.nodes[at];
        node(*folder).children.iter().copied().filter(|&child| node(child).what != What::Rest && !members.contains(&child)).last().map(|kept| (node(kept).held.bytes, node(kept).place))
    }

    /// Several together's: how many things they are, and whether each is a file (a rest among them counted as what
    /// it holds).
    pub fn count(&self, tree: &Tree) -> (u64, bool) {
        match self {
            Item::Node(node) => (1, !tree.nodes[*node].folder()),
            Item::Many { members, .. } => members.iter().fold((0, true), |(count, files), &member| {
                let node = &tree.nodes[member];
                match node.what {
                    What::Rest => (count + node.items, files && node.only_files),
                    _ => (count + 1, files && !node.folder()),
                }
            }),
        }
    }
}

/// A block of the map: what it is, where, at which level, whether it is a ground for what it holds (a first-level
/// folder with its name above), and else, for a folder, the pieces it is drawn as and the folders in it split into
/// pieces, each with all it holds (each where it is within the block, as fractions of it, so they move with it; a
/// folder before those in it).
#[derive(Clone, Debug)]
pub struct Block {
    pub item: Item,
    pub at: Rect,
    pub level: u8,
    pub group: bool,
    pub pieces: Vec<Piece>,
    pub folds: Vec<Piece>,
}

#[derive(Clone, Debug)]
pub struct Piece {
    pub item: Item,
    pub at: Rect,
}

/// The rectangles `weights` (the largest first) take in `rect`, `total` of them filling it, in their order: rows laid
/// along its shorter side, each row taking items while that brings their worst aspect ratio down.
pub fn squarify(weights: &[f64], rect: Rect, total: f64) -> Vec<Rect> {
    let mut out = Vec::with_capacity(weights.len());
    if weights.is_empty() || total <= 0.0 {
        return out;
    }
    let scale = rect.w as f64 * rect.h as f64 / total;
    let (mut x, mut y, mut w, mut h) = (rect.x as f64, rect.y as f64, rect.w as f64, rect.h as f64);
    let mut row: Vec<f64> = Vec::new();
    let (mut sum, mut most, mut least) = (0.0f64, 0.0f64, f64::INFINITY);
    let worst = |sum: f64, most: f64, least: f64, side: f64| {
        let (sum2, side2) = (sum * sum, side * side);
        (side2 * most / sum2).max(sum2 / (side2 * least))
    };
    let lay = |row: &mut Vec<f64>, sum: f64, last: bool, x: &mut f64, y: &mut f64, w: &mut f64, h: &mut f64, out: &mut Vec<Rect>| {
        if *w >= *h {
            // The shorter side is the height: the row is a column at the left.
            let width = if last { *w } else { sum / *h };
            let mut at = *y;
            for (i, area) in row.iter().enumerate() {
                let height = if i == row.len() - 1 { *y + *h - at } else { area / width };
                out.push(Rect { x: *x as f32, y: at as f32, w: width as f32, h: height as f32 });
                at += height;
            }
            *x += width;
            *w -= width;
        } else {
            let height = if last { *h } else { sum / *w };
            let mut at = *x;
            for (i, area) in row.iter().enumerate() {
                let width = if i == row.len() - 1 { *x + *w - at } else { area / height };
                out.push(Rect { x: at as f32, y: *y as f32, w: width as f32, h: height as f32 });
                at += width;
            }
            *y += height;
            *h -= height;
        }
        row.clear();
    };
    for &weight in weights {
        let area = weight * scale;
        let side = w.min(h);
        if !row.is_empty() && worst(sum + area, most.max(area), least.min(area), side) > worst(sum, most, least, side) {
            lay(&mut row, sum, false, &mut x, &mut y, &mut w, &mut h, &mut out);
            (sum, most, least) = (0.0, 0.0, f64::INFINITY);
        }
        row.push(area);
        sum += area;
        most = most.max(area);
        least = least.min(area);
    }
    if !row.is_empty() {
        lay(&mut row, sum, true, &mut x, &mut y, &mut w, &mut h, &mut out);
    }
    out
}

/// What `folder` (a node) holds as it is laid out in `area`: each landing too narrow to draw (less than `side` either
/// way once `gap` is taken out) goes together with all smaller than it into one item, sorted in by its size, and the
/// rest is laid out again. Where that one is itself too narrow it is left out, its little area going to the rest: so
/// none is skipped, leaving a hole beside its neighbours. (What the tree already holds together as too small to draw,
/// its rest, is in that one from the start.)
pub fn gather(tree: &Tree, folder: usize, area: Rect, gap: f32, side: f32) -> Vec<Item> {
    gather_of(tree, folder, &tree.nodes[folder].children, area, gap, side)
}

/// As `gather`, of `children`, some of what `folder` holds (in its order).
fn gather_of(tree: &Tree, folder: usize, children: &[usize], area: Rect, gap: f32, side: f32) -> Vec<Item> {
    let mut kept: Vec<usize> = children.iter().copied().filter(|&child| tree.nodes[child].what != What::Rest).collect();
    let mut small: Vec<usize> = children.iter().copied().filter(|&child| tree.nodes[child].what == What::Rest).collect();
    let mut drop = false;
    loop {
        let together = (small.len() > 1 || small.iter().any(|&member| tree.nodes[member].what == What::Rest)) && !drop;
        let mut list: Vec<Item> = kept.iter().map(|&node| Item::Node(node)).collect();
        if together {
            let many = Item::Many { folder, members: small.clone() };
            let weight = many.weight(tree);
            let at = list.partition_point(|item| item.weight(tree) >= weight);
            list.insert(at, many);
        }
        let weights: Vec<f64> = list.iter().map(|item| item.weight(tree) as f64).collect();
        let total: f64 = weights.iter().sum();
        if total <= 0.0 || list.len() <= 1 {
            return list;
        }
        let placed = squarify(&weights, area, total);
        let narrow: Vec<&Item> = placed.iter().zip(&list).filter(|(at, _)| at.w - gap < side || at.h - gap < side).map(|(_, item)| item).collect();
        if narrow.is_empty() {
            return list;
        }
        let first_narrow = narrow.iter().filter_map(|item| match item {
            Item::Node(node) => kept.iter().position(|kept| kept == node),
            Item::Many { .. } => None,
        });
        match first_narrow.min() {
            Some(at) => {
                let mut moved = kept.split_off(at);
                moved.append(&mut small);
                small = moved;
                drop = false;
            }
            None if !kept.is_empty() => drop = true,
            None => return list,
        }
    }
}

/// What a folder drawn as one block (`item`, at `at`) holds, all the way down, or what several together are: a
/// squarified map of it with a one-DIP line between pieces, a folder too small to split (less than `PIECE_LEAST` either
/// way, or `PIECES_DEEPEST` down) one piece in the colour of what most of it is; and each folder in it split, where it
/// is with all it holds (a folder before those in it). Each where it is within the block, as fractions of it.
pub fn pieces(tree: &Tree, item: &Item, at: Rect) -> (Vec<Piece>, Vec<Piece>) {
    let (mut out, mut folds) = (Vec::new(), Vec::new());
    let relative = |r: Rect| Rect { x: (r.x - at.x) / at.w, y: (r.y - at.y) / at.h, w: r.w / at.w, h: r.h / at.h };
    let (node, members) = match item {
        Item::Node(node) => (*node, None),
        Item::Many { folder, members } => (*folder, Some(members.as_slice())),
    };
    let mut open = vec![(node, at, 0)];
    while let Some((folder, r, depth)) = open.pop() {
        let area = Rect { x: r.x - 0.5, y: r.y - 0.5, w: r.w + 1.0, h: r.h + 1.0 };
        // (Several together: their own map, the first.)
        let items = match members.filter(|_| depth == 0) {
            Some(members) => gather_of(tree, folder, members, area, 1.0, 1.5),
            None => gather(tree, folder, area, 1.0, 1.5),
        };
        if items.is_empty() || depth >= PIECES_DEEPEST {
            out.push(Piece { item: if depth == 0 { item.clone() } else { Item::Node(folder) }, at: relative(r) });
            continue;
        }
        if depth > 0 {
            folds.push(Piece { item: Item::Node(folder), at: relative(r) });
        }
        let weights: Vec<f64> = items.iter().map(|item| item.weight(tree) as f64).collect();
        let total = weights.iter().sum();
        for (item, b) in items.into_iter().zip(squarify(&weights, area, total)) {
            let piece = Rect { x: b.x + 0.5, y: b.y + 0.5, w: b.w - 1.0, h: b.h - 1.0 };
            if piece.w <= 0.3 || piece.h <= 0.3 {
                continue;
            }
            match item {
                Item::Node(inner) if tree.nodes[inner].folder() && piece.w >= PIECE_LEAST && piece.h >= PIECE_LEAST => open.push((inner, piece, depth + 1)),
                item => out.push(Piece { item, at: relative(piece) }),
            }
        }
    }
    (out, folds)
}

/// The map of `tree`'s folder `width` × `height`: what it holds as the first level; each folder among them a ground
/// with its name above what it holds, the second level, where it is large enough (else drawn as its pieces). Gaps are
/// taken out of each block, half on each side, the outer edges flush with the map's. A block's level 2 come after it.
pub fn lay_out(tree: &Tree, width: f32, height: f32) -> Vec<Block> {
    let mut out = Vec::new();
    if tree.nodes.is_empty() {
        return out;
    }
    place(tree, 0, Rect { x: 0.0, y: 0.0, w: width, h: height }, 1, GAP_FIRST, &mut out);
    out
}

fn place(tree: &Tree, folder: usize, area: Rect, level: u8, gap: f32, out: &mut Vec<Block>) {
    let r = Rect { x: area.x - gap / 2.0, y: area.y - gap / 2.0, w: area.w + gap, h: area.h + gap };
    let items = gather(tree, folder, r, gap, SIDE_LEAST);
    let weights: Vec<f64> = items.iter().map(|item| item.weight(tree) as f64).collect();
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return;
    }
    for (item, b) in items.into_iter().zip(squarify(&weights, r, total)) {
        let at = Rect { x: b.x + gap / 2.0, y: b.y + gap / 2.0, w: b.w - gap, h: b.h - gap };
        if at.w <= 0.5 || at.h <= 0.5 {
            continue;
        }
        let split = match &item {
            Item::Node(node) => tree.nodes[*node].folder().then_some(*node),
            Item::Many { .. } => None,
        };
        match (&item, split) {
            (_, Some(node)) if level == 1 && !tree.nodes[node].children.is_empty() && at.w >= GROUP_WIDTH && at.h >= GROUP_HEIGHT => {
                out.push(Block { item, at, level, group: true, pieces: Vec::new(), folds: Vec::new() });
                place(tree, node, Rect { x: at.x + INSET, y: at.y + LABEL, w: at.w - 2.0 * INSET, h: at.h - LABEL - INSET }, 2, GAP_SECOND, out);
            }
            (Item::Many { .. }, _) | (_, Some(_)) => {
                let (pieces, folds) = pieces(tree, &item, at);
                out.push(Block { item, at, level, group: false, pieces, folds });
            }
            _ => out.push(Block { item, at, level, group: false, pieces: Vec::new(), folds: Vec::new() }),
        }
    }
}

/// The block at (`x`, `y`): the deepest there.
pub fn hit(blocks: &[Block], x: f32, y: f32) -> Option<usize> {
    let mut found = None;
    for (at, block) in blocks.iter().enumerate() {
        if block.at.contains(x, y) && found.is_none_or(|found: usize| blocks[found].level < block.level) {
            found = Some(at);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{Node, Role};

    /// A tree of a folder holding `sizes` (a size; a folder: what it holds), its rest `rest` where that is not none.
    fn tree(sizes: &[(u64, &[u64])], rest: u64) -> Tree {
        let mut tree = Tree { nodes: Vec::new(), by_place: Default::default(), rests: Default::default(), after: None, area: 0.0 };
        let node = |place: u32, what: What, bytes: u64| {
            let mut held = Held { bytes, files: 1, ..Held::default() };
            held.kinds[Kind::Video.at()] = bytes;
            Node { place, name: format!("{place}"), folded: String::new(), what, held, children: Vec::new(), opened: true, items: 3, only_files: true }
        };
        tree.nodes.push(node(0, What::Folder(Role::Plain), 0));
        let mut place = 1;
        for (size, inner) in sizes {
            let at = tree.nodes.len();
            tree.nodes[0].children.push(at);
            if inner.is_empty() {
                tree.nodes.push(node(place, What::File { system: false }, *size));
            } else {
                tree.nodes.push(node(place, What::Folder(Role::Plain), inner.iter().sum()));
                for &bytes in inner.iter() {
                    place += 1;
                    let kid = tree.nodes.len();
                    tree.nodes.push(node(place, What::File { system: false }, bytes));
                    tree.nodes[at].children.push(kid);
                }
            }
            place += 1;
        }
        if rest > 0 {
            let at = tree.nodes.len();
            tree.nodes.push(node(place, What::Rest, rest));
            tree.nodes[0].children.push(at);
        }
        let total = tree.nodes[0].children.iter().map(|&kid| tree.nodes[kid].held.bytes).sum();
        tree.nodes[0].held.bytes = total;
        tree
    }

    fn area(r: &Rect) -> f64 {
        r.w as f64 * r.h as f64
    }

    #[test]
    fn squarified_rectangles_fill_the_area_in_proportion_without_overlapping() {
        let weights = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let rect = Rect { x: 10.0, y: 20.0, w: 600.0, h: 400.0 };
        let placed = squarify(&weights, rect, weights.iter().sum());
        assert_eq!(placed.len(), weights.len());
        let total: f64 = placed.iter().map(area).sum();
        assert!((total - area(&rect)).abs() < 1.0, "{total}");
        for (r, weight) in placed.iter().zip(weights) {
            assert!((area(r) / area(&rect) - weight / 24.0).abs() < 1e-3);
            assert!(r.x >= rect.x - 1e-3 && r.y >= rect.y - 1e-3 && r.x + r.w <= rect.x + rect.w + 1e-3 && r.y + r.h <= rect.y + rect.h + 1e-3);
            // (Squarified: none far from square.)
            assert!(r.w.max(r.h) / r.w.min(r.h) < 3.0, "{r:?}");
        }
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                let overlap = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
                let overlap_y = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
                assert!(overlap <= 1e-3 || overlap_y <= 1e-3, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn what_lands_too_narrow_goes_together_with_all_smaller_never_leaving_a_hole() {
        // Three large files, a hundred small ones: the small ones one block, sorted in by its size.
        let mut sizes: Vec<(u64, &[u64])> = vec![(4000, &[]), (3000, &[]), (2000, &[])];
        sizes.extend((0..100).map(|_| (10, &[][..])));
        let tree = tree(&sizes, 0);
        let items = gather(&tree, 0, Rect { x: 0.0, y: 0.0, w: 300.0, h: 200.0 }, GAP_FIRST, SIDE_LEAST);
        assert_eq!(items.len(), 4);
        assert_eq!(items[..3], [Item::Node(1), Item::Node(2), Item::Node(3)]);
        let Item::Many { folder, members } = &items[3] else { panic!("{items:?}") };
        assert_eq!((*folder, members.len()), (0, 100));
        assert_eq!(items[3].count(&tree), (100, true));
        // They start past the smallest drawn on its own, and are drawn as their pieces, filling their block.
        assert_eq!(items[3].bound(&tree), Some((2000, tree.nodes[3].place)));
        assert_eq!(Item::Node(1).bound(&tree), None);
        let blocks = lay_out(&tree, 300.0, 200.0);
        let many = blocks.iter().find(|block| matches!(block.item, Item::Many { .. })).unwrap();
        assert_eq!(many.pieces.len(), 100);
        assert!(many.pieces.iter().all(|piece| matches!(piece.item, Item::Node(node) if node > 3)));
        let covered: f32 = many.pieces.iter().map(|piece| piece.at.w * piece.at.h).sum();
        assert!(covered > 0.6 && covered <= 1.0 + 1e-3, "{covered}");
        // The tree's rest is always among them, counted as what it holds.
        let tree = self::tree(&[(4000, &[]), (3000, &[])], 500);
        let items = gather(&tree, 0, Rect { x: 0.0, y: 0.0, w: 300.0, h: 200.0 }, GAP_FIRST, SIDE_LEAST);
        assert_eq!(items[2].count(&tree), (3, true));
        // Laid out, the blocks fill the map but for the gaps between them.
        let blocks = lay_out(&tree, 300.0, 200.0);
        let covered: f64 = blocks.iter().map(|block| (block.at.w + GAP_FIRST) as f64 * (block.at.h + GAP_FIRST) as f64).sum();
        assert!((covered - 306.0 * 206.0).abs() < 2.0, "{covered}");
    }

    #[test]
    fn a_large_first_level_folder_shows_what_it_holds_and_a_small_one_is_drawn_in_pieces() {
        let tree = tree(&[(0, &[3000, 2000, 1000]), (0, &[600, 300, 100])], 0);
        let blocks = lay_out(&tree, 400.0, 300.0);
        let large = blocks.iter().find(|block| block.item == Item::Node(1)).unwrap();
        assert!(large.group && large.pieces.is_empty());
        let inner: Vec<&Block> = blocks.iter().filter(|block| block.level == 2).collect();
        assert_eq!(inner.len(), 3);
        for block in &inner {
            assert!(block.at.x >= large.at.x + INSET - 1e-3 && block.at.y >= large.at.y + LABEL - 1e-3);
            assert!(block.at.x + block.at.w <= large.at.x + large.at.w - INSET + 1e-3 && block.at.y + block.at.h <= large.at.y + large.at.h - INSET + 1e-3);
        }
        // What it holds fills the ground but for the margin: its sides and foot the same way in from the ground's.
        let most = |edge: fn(&&Block) -> f32| inner.iter().map(edge).fold(f32::MIN, f32::max);
        assert!((most(|block| block.at.y + block.at.h) - (large.at.y + large.at.h - INSET)).abs() < 1e-3);
        assert!((most(|block| block.at.x + block.at.w) - (large.at.x + large.at.w - INSET)).abs() < 1e-3);
        assert!((inner.iter().map(|block| block.at.x).fold(f32::MAX, f32::min) - (large.at.x + INSET)).abs() < 1e-3);
        let small = blocks.iter().find(|block| block.item == Item::Node(5)).unwrap();
        assert!(!small.group);
        assert_eq!(small.pieces.len(), 3);
        assert!(small.folds.is_empty());
        let covered: f32 = small.pieces.iter().map(|piece| piece.at.w * piece.at.h).sum();
        assert!(covered > 0.8 && covered <= 1.0, "{covered}");
        // A folder in one drawn as pieces is kept with all it holds, its pieces inside it.
        let mut tree = tree;
        let (inner_folder, files) = (tree.nodes[5].children[0], tree.nodes.len());
        for bytes in [400, 200] {
            let mut file = tree.nodes[inner_folder].clone();
            (file.place, file.held.bytes) = (100 + tree.nodes.len() as u32, bytes);
            tree.nodes.push(file);
        }
        tree.nodes[inner_folder].what = What::Folder(Role::Plain);
        tree.nodes[inner_folder].children = vec![files, files + 1];
        let (pieces, folds) = super::pieces(&tree, &Item::Node(5), Rect { x: 0.0, y: 0.0, w: 300.0, h: 200.0 });
        assert_eq!(folds.iter().map(|fold| &fold.item).collect::<Vec<_>>(), [&Item::Node(inner_folder)]);
        let fold = folds[0].at;
        for node in [files, files + 1] {
            let piece = pieces.iter().find(|piece| piece.item == Item::Node(node)).unwrap().at;
            assert!(piece.x >= fold.x - 1e-4 && piece.y >= fold.y - 1e-4 && piece.x + piece.w <= fold.x + fold.w + 1e-4 && piece.y + piece.h <= fold.y + fold.h + 1e-4, "{piece:?} {fold:?}");
        }
        assert!(!pieces.iter().any(|piece| piece.item == Item::Node(inner_folder)));
        // What is hit is the deepest there.
        let first = inner[0];
        assert_eq!(hit(&blocks, first.at.x + 1.0, first.at.y + 1.0).map(|at| &blocks[at].item), Some(&first.item));
        assert_eq!(hit(&blocks, large.at.x + 1.0, large.at.y + 1.0).map(|at| &blocks[at].item), Some(&Item::Node(1)));
    }
}
