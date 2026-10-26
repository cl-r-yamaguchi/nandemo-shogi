//! 駒のデータ（`data/pieces.toml`）の読み込み・検証と、移動量の計算。
//! 仕様は docs/specs/pieces.md と docs/specs/game-rules.md の 4 章。

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::BOARD_SIZE;
use crate::geometry::{Facing, Pos, Rect};

/// 城の駒の id。ルールが特別に扱う唯一の駒。
pub const CASTLE_ID: &str = "castle";

/// 駒の種類を表す番号（カタログ内の位置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct KindId(pub usize);

/// 移動範囲の方向。（右, 前）の単位ベクトルに対応する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    Front,
    Back,
    Left,
    Right,
    FrontLeft,
    FrontRight,
    BackLeft,
    BackRight,
}

impl Direction {
    /// （右, 前）の単位ベクトル。
    pub const fn unit(self) -> (i32, i32) {
        match self {
            Direction::Front => (0, 1),
            Direction::Back => (0, -1),
            Direction::Left => (-1, 0),
            Direction::Right => (1, 0),
            Direction::FrontLeft => (-1, 1),
            Direction::FrontRight => (1, 1),
            Direction::BackLeft => (-1, -1),
            Direction::BackRight => (1, -1),
        }
    }
}

/// 方向と距離の部品: 指定の方向に、`min` から `max` マス。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ray {
    pub dir: Direction,
    pub min: i32,
    pub max: i32,
}

/// 移動範囲の部品（docs/specs/pieces.md の 2 章）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum MovePart {
    /// 指定した（右, 前）へ動ける。
    Offsets(Vec<[i32; 2]>),
    /// 前後 `front`・左右 `side` の長方形の、自分の位置を除くすべて。
    Rect { front: i32, side: i32 },
    /// 方向と距離。
    Rays(Vec<Ray>),
    /// 他の駒か盤の端まで直進する。
    Charge(Direction),
}

/// 駒の種類ごとの定義。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PieceDef {
    pub id: String,
    pub name: String,
    /// 大きさ（横, 縦）。向きが北のときの値。
    pub size: [u32; 2],
    pub attack: u32,
    pub hp: u32,
    /// ドローでの出現重み。0 は引かれない。
    pub weight: u32,
    #[serde(default)]
    pub promotes_to: Option<String>,
    #[serde(default)]
    pub movement: Vec<MovePart>,
}

impl PieceDef {
    pub fn is_castle(&self) -> bool {
        self.id == CASTLE_ID
    }

    /// 動けない駒か。
    pub fn is_immobile(&self) -> bool {
        self.movement.is_empty()
    }

    /// アンカーと向きから占有範囲を作る（R-02・R-03）。
    pub fn footprint(&self, anchor: Pos, facing: Facing) -> Rect {
        Rect::footprint(anchor, facing, (self.size[0], self.size[1]))
    }

    /// 向きを反映した大きさ（横, 縦）。R-02。
    pub fn oriented_size(&self, facing: Facing) -> (u32, u32) {
        facing.oriented_size((self.size[0], self.size[1]))
    }

    /// アンカーを動かせる量（dx, dy）の集まり。直進の部品は含まない。
    /// （0, 0）は含まない。重複は取り除く。盤の内外・他の駒との重なりは、ここでは見ない。
    pub fn displacements(&self, facing: Facing) -> Vec<(i32, i32)> {
        let mut set = BTreeSet::new();
        let mut add = |right: i32, front: i32| {
            if (right, front) != (0, 0) {
                set.insert(facing.to_delta(right, front));
            }
        };
        for part in &self.movement {
            match part {
                MovePart::Offsets(offsets) => offsets.iter().for_each(|[r, f]| add(*r, *f)),
                MovePart::Rect { front, side } => {
                    for f in -front..=*front {
                        for r in -side..=*side {
                            add(r, f);
                        }
                    }
                }
                MovePart::Rays(rays) => {
                    for ray in rays {
                        let (ur, uf) = ray.dir.unit();
                        for n in ray.min..=ray.max {
                            add(ur * n, uf * n);
                        }
                    }
                }
                MovePart::Charge(_) => {}
            }
        }
        set.into_iter().collect()
    }

    /// 直進の駒の、盤上の進行方向（dx, dy）。直進しない駒は `None`。
    pub fn charge_delta(&self, facing: Facing) -> Option<(i32, i32)> {
        self.movement.iter().find_map(|part| match part {
            MovePart::Charge(dir) => {
                let (r, f) = dir.unit();
                Some(facing.to_delta(r, f))
            }
            _ => None,
        })
    }
}

/// 駒データの読み込み・検証のエラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    Parse(String),
    DuplicateId(String),
    UnknownPromotion {
        piece: String,
        target: String,
    },
    InvalidSize(String),
    InvalidHp(String),
    InvalidMovement {
        piece: String,
        reason: &'static str,
    },
    MissingCastle,
    NoDrawablePiece,
    /// 城は、出現重み 0・動かない・成らない・成り先にならない必要がある。
    InvalidCastle(&'static str),
    InvalidPromotion {
        piece: String,
        reason: &'static str,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(message) => write!(f, "駒データを読み込めません: {message}"),
            Self::DuplicateId(id) => write!(f, "駒の id が重複しています: {id}"),
            Self::UnknownPromotion { piece, target } => {
                write!(f, "{piece} の成り先 {target} が存在しません")
            }
            Self::InvalidSize(id) => write!(f, "{id} の大きさが不正です（1 以上が必要）"),
            Self::InvalidHp(id) => write!(f, "{id} の HP が不正です（1 以上が必要）"),
            Self::InvalidMovement { piece, reason } => {
                write!(f, "{piece} の移動範囲が不正です: {reason}")
            }
            Self::MissingCastle => write!(f, "城（{CASTLE_ID}）の定義がありません"),
            Self::NoDrawablePiece => write!(f, "ドローで引ける駒（出現重み 1 以上）がありません"),
            Self::InvalidCastle(reason) => write!(f, "城の定義が不正です: {reason}"),
            Self::InvalidPromotion { piece, reason } => {
                write!(f, "{piece} の成りの定義が不正です: {reason}")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    piece: Vec<PieceDef>,
}

/// 駒の定義の一覧。読み込み時に検証済みで、以降は不変。
#[derive(Debug, Clone)]
pub struct Catalog {
    defs: Vec<PieceDef>,
    by_id: HashMap<String, KindId>,
}

impl Catalog {
    /// リポジトリの `data/pieces.toml`（コンパイル時に埋め込み）から作る。
    pub fn builtin() -> Result<Self, CatalogError> {
        Self::from_toml(include_str!("../../../data/pieces.toml"))
    }

    pub fn from_toml(source: &str) -> Result<Self, CatalogError> {
        let file: CatalogFile =
            toml::from_str(source).map_err(|e| CatalogError::Parse(e.to_string()))?;
        Self::new(file.piece)
    }

    pub fn new(defs: Vec<PieceDef>) -> Result<Self, CatalogError> {
        let mut by_id = HashMap::new();
        for (index, def) in defs.iter().enumerate() {
            if by_id.insert(def.id.clone(), KindId(index)).is_some() {
                return Err(CatalogError::DuplicateId(def.id.clone()));
            }
        }
        let catalog = Self { defs, by_id };
        catalog.validate()?;
        Ok(catalog)
    }

    fn validate(&self) -> Result<(), CatalogError> {
        if !self.by_id.contains_key(CASTLE_ID) {
            return Err(CatalogError::MissingCastle);
        }
        if !self.defs.iter().any(|d| d.weight > 0 && !d.is_castle()) {
            return Err(CatalogError::NoDrawablePiece);
        }
        for def in &self.defs {
            if def.size.iter().any(|&n| n == 0 || n > BOARD_SIZE) {
                return Err(CatalogError::InvalidSize(def.id.clone()));
            }
            if def.is_castle() {
                if def.weight != 0 {
                    return Err(CatalogError::InvalidCastle("出現重みは 0 にしてください"));
                }
                if !def.is_immobile() {
                    return Err(CatalogError::InvalidCastle("城は動けません"));
                }
                if def.promotes_to.is_some() {
                    return Err(CatalogError::InvalidCastle("城は成りません"));
                }
            }
            if def.hp == 0 {
                return Err(CatalogError::InvalidHp(def.id.clone()));
            }
            if let Some(target) = &def.promotes_to {
                let Some(&kind) = self.by_id.get(target) else {
                    return Err(CatalogError::UnknownPromotion {
                        piece: def.id.clone(),
                        target: target.clone(),
                    });
                };
                let target_def = &self.defs[kind.0];
                let reason = if target == &def.id {
                    Some("自分自身には成れません")
                } else if target_def.is_castle() {
                    Some("城には成れません")
                } else if target_def.promotes_to.is_some() {
                    Some("成り先が、さらに成ることはできません")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    return Err(CatalogError::InvalidPromotion {
                        piece: def.id.clone(),
                        reason,
                    });
                }
            }
            validate_movement(def)?;
        }
        Ok(())
    }

    pub fn get(&self, kind: KindId) -> Option<&PieceDef> {
        self.defs.get(kind.0)
    }

    pub fn kind_of(&self, id: &str) -> Option<KindId> {
        self.by_id.get(id).copied()
    }

    pub fn castle(&self) -> KindId {
        self.by_id[CASTLE_ID]
    }

    /// 成り先の種類。成らない駒は `None`。
    pub fn promotion(&self, kind: KindId) -> Option<KindId> {
        let target = self.get(kind)?.promotes_to.as_ref()?;
        self.kind_of(target)
    }

    pub fn iter(&self) -> impl Iterator<Item = (KindId, &PieceDef)> {
        self.defs.iter().enumerate().map(|(i, d)| (KindId(i), d))
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

fn validate_movement(def: &PieceDef) -> Result<(), CatalogError> {
    let bad = |reason| CatalogError::InvalidMovement {
        piece: def.id.clone(),
        reason,
    };
    // 盤の外へ出るだけの移動量は意味がなく、巨大な値は計算を壊すため、盤の大きさで上限を決める
    let limit = BOARD_SIZE as i32;
    let mut charges = 0;
    for part in &def.movement {
        match part {
            MovePart::Offsets(offsets) => {
                if offsets.is_empty() {
                    return Err(bad("相対位置が空です"));
                }
                if offsets.contains(&[0, 0]) {
                    return Err(bad("相対位置に（0, 0）は指定できません"));
                }
                if offsets.iter().flatten().any(|n| n.abs() > limit) {
                    return Err(bad("相対位置が盤の大きさを超えています"));
                }
            }
            MovePart::Rect { front, side } => {
                if *front < 0 || *side < 0 || (*front == 0 && *side == 0) {
                    return Err(bad("範囲の前後・左右は 0 以上で、両方が 0 にはできません"));
                }
                if *front > limit || *side > limit {
                    return Err(bad("範囲が盤の大きさを超えています"));
                }
            }
            MovePart::Rays(rays) => {
                if rays.is_empty() {
                    return Err(bad("方向と距離が空です"));
                }
                if rays.iter().any(|r| r.min < 1 || r.max < r.min) {
                    return Err(bad("距離は 1 以上で、最大は最小以上が必要です"));
                }
                if rays.iter().any(|r| r.max > limit) {
                    return Err(bad("距離が盤の大きさを超えています"));
                }
            }
            MovePart::Charge(_) => charges += 1,
        }
    }
    if charges > 1 || (charges == 1 && def.movement.len() > 1) {
        return Err(bad("直進は、1 つだけで、ほかの部品と組み合わせられません"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::builtin().expect("pieces.toml が読み込めること")
    }

    fn def<'a>(catalog: &'a Catalog, id: &str) -> &'a PieceDef {
        catalog.get(catalog.kind_of(id).unwrap()).unwrap()
    }

    #[test]
    fn builtin_contains_the_castle() {
        let catalog = catalog();
        assert!(def(&catalog, CASTLE_ID).is_castle());
    }

    #[test]
    fn castle_is_2x2_hp10_attack0_and_immobile() {
        let catalog = catalog();
        let castle = def(&catalog, CASTLE_ID);
        assert_eq!((castle.size, castle.attack, castle.hp), ([2, 2], 0, 10));
        assert!(castle.is_immobile());
        assert_eq!(castle.weight, 0);
    }

    #[test]
    fn promotions_are_the_four_kid_to_adult_pairs() {
        let catalog = catalog();
        let promoted = |id: &str| {
            catalog
                .promotion(catalog.kind_of(id).unwrap())
                .map(|k| catalog.get(k).unwrap().id.as_str())
        };
        assert_eq!(promoted("kid-elephant"), Some("elephant"));
        assert_eq!(promoted("foal"), Some("horse"));
        assert_eq!(promoted("kid-lion"), Some("lion"));
        assert_eq!(promoted("baby-penguin"), Some("jet-penguin"));
        let count = catalog
            .iter()
            .filter(|(k, _)| catalog.promotion(*k).is_some())
            .count();
        assert_eq!(count, 4);
    }

    #[test]
    fn displacement_counts_match_the_spec() {
        let catalog = catalog();
        let count = |id: &str| def(&catalog, id).displacements(Facing::North).len();
        assert_eq!(count("kid-elephant"), 8);
        assert_eq!(count("elephant"), 24);
        assert_eq!(count("foal"), 2);
        assert_eq!(count("horse"), 16);
        assert_eq!(count("kid-lion"), 1);
        assert_eq!(count("lion"), 8);
        assert_eq!(count("orca"), 14);
        assert_eq!(count("baby-penguin"), 1);
        assert_eq!(count("bear"), 4);
        assert_eq!(count("deer"), 8);
        assert_eq!(count("jet-penguin"), 0);
        assert_eq!(count(CASTLE_ID), 0);
    }

    #[test]
    fn foal_moves_like_a_knight_relative_to_facing() {
        let catalog = catalog();
        let foal = def(&catalog, "foal");
        assert_eq!(foal.displacements(Facing::North), [(-1, -2), (1, -2)]);
        assert_eq!(foal.displacements(Facing::East), [(2, -1), (2, 1)]);
        assert_eq!(foal.displacements(Facing::South), [(-1, 2), (1, 2)]);
        assert_eq!(foal.displacements(Facing::West), [(-2, -1), (-2, 1)]);
    }

    #[test]
    fn horse_covers_exactly_the_ring_at_distance_two() {
        let catalog = catalog();
        for facing in Facing::ALL {
            let d = def(&catalog, "horse").displacements(facing);
            assert!(d.iter().all(|(x, y)| x.abs().max(y.abs()) == 2));
        }
    }

    #[test]
    fn orca_rect_is_two_forward_and_one_sideways() {
        let catalog = catalog();
        let north = def(&catalog, "orca").displacements(Facing::North);
        assert!(north.iter().all(|(x, y)| x.abs() <= 1 && y.abs() <= 2));
        assert!(north.contains(&(1, -2)) && north.contains(&(-1, 2)));
        let east = def(&catalog, "orca").displacements(Facing::East);
        assert!(east.iter().all(|(x, y)| x.abs() <= 2 && y.abs() <= 1));
    }

    #[test]
    fn deer_moves_only_diagonally_up_to_two() {
        let catalog = catalog();
        let d = def(&catalog, "deer").displacements(Facing::North);
        assert!(
            d.iter()
                .all(|(x, y)| x.abs() == y.abs() && (1..=2).contains(&x.abs()))
        );
    }

    #[test]
    fn displacements_never_contain_the_origin() {
        let catalog = catalog();
        for (_, d) in catalog.iter() {
            for facing in Facing::ALL {
                assert!(!d.displacements(facing).contains(&(0, 0)));
            }
        }
    }

    #[test]
    fn r53_jet_penguin_charges_forward() {
        let catalog = catalog();
        let jet = def(&catalog, "jet-penguin");
        assert_eq!(jet.charge_delta(Facing::North), Some((0, -1)));
        assert_eq!(jet.charge_delta(Facing::East), Some((1, 0)));
        assert_eq!(def(&catalog, "lion").charge_delta(Facing::North), None);
    }

    #[test]
    fn only_jet_penguin_charges() {
        let catalog = catalog();
        let chargers: Vec<_> = catalog
            .iter()
            .filter(|(_, d)| d.charge_delta(Facing::North).is_some())
            .map(|(_, d)| d.id.as_str())
            .collect();
        assert_eq!(chargers, ["jet-penguin"]);
    }

    #[test]
    fn drawable_pieces_exclude_the_castle() {
        let catalog = catalog();
        assert!(
            catalog
                .iter()
                .filter(|(_, d)| d.weight > 0)
                .all(|(_, d)| !d.is_castle())
        );
    }

    #[test]
    fn orca_is_one_by_two_and_rotates_with_facing() {
        let catalog = catalog();
        let orca = def(&catalog, "orca");
        assert_eq!(
            orca.footprint(Pos::new(3, 3), Facing::North),
            Rect {
                x: 3,
                y: 3,
                w: 1,
                h: 2
            }
        );
        assert_eq!(
            orca.footprint(Pos::new(3, 3), Facing::East),
            Rect {
                x: 3,
                y: 3,
                w: 2,
                h: 1
            }
        );
    }

    const MINIMAL: &str = r#"
[[piece]]
id = "castle"
name = "城"
size = [2, 2]
attack = 0
hp = 10
weight = 0

[[piece]]
id = "pawn"
name = "歩"
size = [1, 1]
attack = 1
hp = 1
weight = 1
"#;

    #[test]
    fn minimal_data_is_accepted() {
        assert!(Catalog::from_toml(MINIMAL).is_ok());
    }

    #[test]
    fn rejects_unparsable_data() {
        assert!(matches!(
            Catalog::from_toml("piece = 1"),
            Err(CatalogError::Parse(_))
        ));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let source = format!(
            "{MINIMAL}\n[[piece]]\nid = \"pawn\"\nname = \"x\"\nsize = [1, 1]\nattack = 1\nhp = 1\nweight = 1\n"
        );
        assert_eq!(
            Catalog::from_toml(&source).unwrap_err(),
            CatalogError::DuplicateId("pawn".into())
        );
    }

    #[test]
    fn rejects_unknown_promotion_target() {
        let source = MINIMAL.replace("weight = 1", "weight = 1\npromotes_to = \"ghost\"");
        assert!(matches!(
            Catalog::from_toml(&source),
            Err(CatalogError::UnknownPromotion { .. })
        ));
    }

    #[test]
    fn rejects_zero_size_and_zero_hp() {
        let zero_size = MINIMAL.replace("size = [1, 1]", "size = [0, 1]");
        assert!(matches!(
            Catalog::from_toml(&zero_size),
            Err(CatalogError::InvalidSize(_))
        ));
        let zero_hp = MINIMAL.replace("hp = 1\nweight", "hp = 0\nweight");
        assert!(matches!(
            Catalog::from_toml(&zero_hp),
            Err(CatalogError::InvalidHp(_))
        ));
    }

    #[test]
    fn rejects_missing_castle_and_missing_drawable() {
        let no_castle = MINIMAL.replace("id = \"castle\"", "id = \"keep\"");
        assert_eq!(
            Catalog::from_toml(&no_castle).unwrap_err(),
            CatalogError::MissingCastle
        );
        let no_draw = MINIMAL.replace("weight = 1", "weight = 0");
        assert_eq!(
            Catalog::from_toml(&no_draw).unwrap_err(),
            CatalogError::NoDrawablePiece
        );
    }

    #[test]
    fn rejects_invalid_movement() {
        let bad = |movement: &str| {
            let source =
                MINIMAL.replace("weight = 1", &format!("weight = 1\nmovement = {movement}"));
            Catalog::from_toml(&source).unwrap_err()
        };
        assert!(matches!(
            bad("[{ offsets = [[0, 0]] }]"),
            CatalogError::InvalidMovement { .. }
        ));
        assert!(matches!(
            bad("[{ rect = { front = 0, side = 0 } }]"),
            CatalogError::InvalidMovement { .. }
        ));
        assert!(matches!(
            bad("[{ rays = [{ dir = \"front\", min = 2, max = 1 }] }]"),
            CatalogError::InvalidMovement { .. }
        ));
        assert!(matches!(
            bad("[{ charge = \"front\" }, { charge = \"back\" }]"),
            CatalogError::InvalidMovement { .. }
        ));
    }

    #[test]
    fn rejects_unknown_keys() {
        let typo = MINIMAL.replace("weight = 1", "weight = 1\nwieght = 2");
        assert!(matches!(
            Catalog::from_toml(&typo),
            Err(CatalogError::Parse(_))
        ));
        let bad_part = MINIMAL.replace("weight = 1", "weight = 1\nmovement = [{ jump = 1 }]");
        assert!(matches!(
            Catalog::from_toml(&bad_part),
            Err(CatalogError::Parse(_))
        ));
    }

    #[test]
    fn rejects_oversized_values() {
        let huge_size = MINIMAL.replace("size = [1, 1]", "size = [4294967295, 1]");
        assert!(matches!(
            Catalog::from_toml(&huge_size),
            Err(CatalogError::InvalidSize(_))
        ));
        let huge_rect = MINIMAL.replace(
            "weight = 1",
            "weight = 1\nmovement = [{ rect = { front = 2000000000, side = 1 } }]",
        );
        assert!(matches!(
            Catalog::from_toml(&huge_rect),
            Err(CatalogError::InvalidMovement { .. })
        ));
        let huge_ray = MINIMAL.replace(
            "weight = 1",
            "weight = 1\nmovement = [{ rays = [{ dir = \"front\", min = 1, max = 2147483647 }] }]",
        );
        assert!(matches!(
            Catalog::from_toml(&huge_ray),
            Err(CatalogError::InvalidMovement { .. })
        ));
    }

    #[test]
    fn rejects_castle_that_can_be_drawn_or_moves_or_promotes() {
        let drawable = MINIMAL.replacen("weight = 0", "weight = 5", 1);
        assert!(matches!(
            Catalog::from_toml(&drawable),
            Err(CatalogError::InvalidCastle(_))
        ));
        let moving = MINIMAL.replacen(
            "weight = 0",
            "weight = 0\nmovement = [{ rect = { front = 1, side = 1 } }]",
            1,
        );
        assert!(matches!(
            Catalog::from_toml(&moving),
            Err(CatalogError::InvalidCastle(_))
        ));
        let promoting = MINIMAL.replacen("weight = 0", "weight = 0\npromotes_to = \"pawn\"", 1);
        assert!(matches!(
            Catalog::from_toml(&promoting),
            Err(CatalogError::InvalidCastle(_))
        ));
    }

    #[test]
    fn rejects_bad_promotion_targets() {
        let to_castle = MINIMAL.replace("weight = 1", "weight = 1\npromotes_to = \"castle\"");
        assert!(matches!(
            Catalog::from_toml(&to_castle),
            Err(CatalogError::InvalidPromotion { .. })
        ));
        let to_self = MINIMAL.replace("weight = 1", "weight = 1\npromotes_to = \"pawn\"");
        assert!(matches!(
            Catalog::from_toml(&to_self),
            Err(CatalogError::InvalidPromotion { .. })
        ));
    }

    #[test]
    fn rejects_promotion_chains() {
        let chain = format!(
            "{MINIMAL}\n[[piece]]\nid = \"a\"\nname = \"a\"\nsize = [1, 1]\nattack = 1\nhp = 1\nweight = 1\npromotes_to = \"b\"\n\n[[piece]]\nid = \"b\"\nname = \"b\"\nsize = [1, 1]\nattack = 1\nhp = 1\nweight = 1\npromotes_to = \"pawn\"\n"
        );
        assert!(matches!(
            Catalog::from_toml(&chain),
            Err(CatalogError::InvalidPromotion { .. })
        ));
    }

    #[test]
    fn rejects_charge_combined_with_other_parts() {
        let combined = MINIMAL.replace(
            "weight = 1",
            "weight = 1\nmovement = [{ charge = \"front\" }, { rect = { front = 1, side = 1 } }]",
        );
        assert!(matches!(
            Catalog::from_toml(&combined),
            Err(CatalogError::InvalidMovement { .. })
        ));
    }

    #[test]
    fn castle_only_data_has_no_drawable_piece() {
        let castle_only = MINIMAL.split("[[piece]]\nid = \"pawn\"").next().unwrap();
        assert_eq!(
            Catalog::from_toml(castle_only).unwrap_err(),
            CatalogError::NoDrawablePiece
        );
    }
}
