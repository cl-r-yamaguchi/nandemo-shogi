//! 盤の幾何: 座標、向き、占有範囲。仕様は docs/specs/game-rules.md の 2 章。

use serde::{Deserialize, Serialize};

/// 盤上のマスの座標。x は左から右、y は上から下（北が y の小さいほう）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
}

impl Pos {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// 駒の向き。移動範囲の「前」の基準。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Facing {
    North,
    East,
    South,
    West,
}

impl Facing {
    pub const ALL: [Facing; 4] = [Facing::North, Facing::East, Facing::South, Facing::West];

    /// R-01: 向きを基準にした（右, 前）の移動量を、盤上の移動量（dx, dy）にする。
    pub const fn to_delta(self, right: i32, front: i32) -> (i32, i32) {
        match self {
            Facing::North => (right, -front),
            Facing::East => (front, right),
            Facing::South => (-right, front),
            Facing::West => (-front, -right),
        }
    }

    /// R-02: 大きさ（横, 縦）を、向きを反映した大きさにする。東西では横と縦を入れ替える。
    pub const fn oriented_size(self, (w, h): (u32, u32)) -> (u32, u32) {
        match self {
            Facing::North | Facing::South => (w, h),
            Facing::East | Facing::West => (h, w),
        }
    }
}

/// 長方形のマスの集まり（占有範囲）。左上のマス（アンカー）と、横・縦のマス数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    /// R-03: アンカーと、向きを反映した大きさから、占有範囲を作る。
    pub fn footprint(anchor: Pos, facing: Facing, size: (u32, u32)) -> Self {
        let (w, h) = facing.oriented_size(size);
        Self {
            x: anchor.x,
            y: anchor.y,
            w: w as i32,
            h: h as i32,
        }
    }

    /// 占有範囲のすべてのマス。
    pub fn cells(&self) -> impl Iterator<Item = Pos> + '_ {
        (self.y..self.y + self.h)
            .flat_map(move |y| (self.x..self.x + self.w).map(move |x| Pos { x, y }))
    }

    /// すべてのマスが、一辺 `board_size` の盤の内側にあるか（R-04 の 1）。
    pub fn is_inside(&self, board_size: u32) -> bool {
        let size = board_size as i32;
        self.x >= 0 && self.y >= 0 && self.x + self.w <= size && self.y + self.h <= size
    }

    /// 重なっているマスの数（R-54 のダメージ計算に使う）。
    pub fn overlap_area(&self, other: &Rect) -> i32 {
        let w = (self.x + self.w).min(other.x + other.w) - self.x.max(other.x);
        let h = (self.y + self.h).min(other.y + other.h) - self.y.max(other.y);
        w.max(0) * h.max(0)
    }

    /// 1 マスでも重なっているか（R-04 の 2）。
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.overlap_area(other) > 0
    }

    /// アンカーを（dx, dy）だけ動かした占有範囲。
    pub fn translated(&self, dx: i32, dy: i32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r01_front_and_right_follow_facing() {
        // 前 1 マス
        assert_eq!(Facing::North.to_delta(0, 1), (0, -1));
        assert_eq!(Facing::East.to_delta(0, 1), (1, 0));
        assert_eq!(Facing::South.to_delta(0, 1), (0, 1));
        assert_eq!(Facing::West.to_delta(0, 1), (-1, 0));
        // 右 1 マス
        assert_eq!(Facing::North.to_delta(1, 0), (1, 0));
        assert_eq!(Facing::East.to_delta(1, 0), (0, 1));
        assert_eq!(Facing::South.to_delta(1, 0), (-1, 0));
        assert_eq!(Facing::West.to_delta(1, 0), (0, -1));
    }

    #[test]
    fn r01_rotation_is_a_bijection_on_deltas() {
        for facing in Facing::ALL {
            let mut seen = std::collections::BTreeSet::new();
            for right in -2..=2 {
                for front in -2..=2 {
                    assert!(seen.insert(facing.to_delta(right, front)));
                }
            }
        }
    }

    #[test]
    fn r02_size_swaps_for_east_and_west() {
        assert_eq!(Facing::North.oriented_size((1, 2)), (1, 2));
        assert_eq!(Facing::South.oriented_size((1, 2)), (1, 2));
        assert_eq!(Facing::East.oriented_size((1, 2)), (2, 1));
        assert_eq!(Facing::West.oriented_size((1, 2)), (2, 1));
        assert_eq!(Facing::East.oriented_size((2, 2)), (2, 2));
    }

    #[test]
    fn r03_footprint_starts_at_anchor() {
        let rect = Rect::footprint(Pos::new(3, 4), Facing::East, (1, 2));
        assert_eq!(
            rect,
            Rect {
                x: 3,
                y: 4,
                w: 2,
                h: 1
            }
        );
        let cells: Vec<_> = rect.cells().collect();
        assert_eq!(cells, [Pos::new(3, 4), Pos::new(4, 4)]);
    }

    #[test]
    fn r04_inside_board_checks_every_cell() {
        let big = |x, y| Rect::footprint(Pos::new(x, y), Facing::North, (2, 2));
        assert!(big(14, 14).is_inside(16));
        assert!(!big(15, 14).is_inside(16));
        assert!(!big(14, 15).is_inside(16));
        assert!(!big(-1, 0).is_inside(16));
    }

    #[test]
    fn overlap_area_counts_shared_cells() {
        let big = Rect::footprint(Pos::new(0, 0), Facing::North, (2, 2));
        let small = |x, y| Rect::footprint(Pos::new(x, y), Facing::North, (1, 1));
        assert_eq!(big.overlap_area(&small(1, 1)), 1);
        assert_eq!(big.overlap_area(&small(2, 0)), 0);
        let other = Rect::footprint(Pos::new(1, 0), Facing::North, (2, 2));
        assert_eq!(big.overlap_area(&other), 2);
        assert!(big.overlaps(&other));
        assert!(!big.overlaps(&small(2, 2)));
    }

    #[test]
    fn translated_moves_the_anchor_only() {
        let rect = Rect::footprint(Pos::new(1, 1), Facing::North, (2, 2));
        assert_eq!(
            rect.translated(2, -1),
            Rect {
                x: 3,
                y: 0,
                w: 2,
                h: 2
            }
        );
    }
}
