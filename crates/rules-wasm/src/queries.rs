//! 問い合わせの本体。wasm に依存せず、文字列（JSON）を受け取って、文字列を返す。

use std::sync::OnceLock;

use protocol::StateView;
use rules::action::{
    Action, apply, legal_actions as rules_legal_actions,
    legal_placements as rules_legal_placements, legal_targets as rules_legal_targets,
};
use rules::pieces::{Catalog, KindId};
use rules::state::{GameState, PieceId};
use rules::threats::threatened_pieces;
use serde::Serialize;

/// 盤の大きさの上限。これを超える写しは、受け付けない（全マスを調べる問い合わせが、重くなるため）。
const MAX_BOARD_SIZE: u32 = 64;

fn catalog() -> Result<&'static Catalog, String> {
    static CATALOG: OnceLock<Result<Catalog, String>> = OnceLock::new();
    CATALOG
        .get_or_init(|| Catalog::builtin().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(Clone::clone)
}

fn state_of(json: &str) -> Result<(&'static Catalog, GameState), String> {
    let catalog = catalog()?;
    let view: StateView =
        serde_json::from_str(json).map_err(|e| format!("盤面を読めません: {e}"))?;
    if view.board_size == 0 || view.board_size > MAX_BOARD_SIZE {
        return Err(format!("盤の大きさが範囲外です: {}", view.board_size));
    }
    let state = view
        .to_state(catalog)
        .map_err(|e| format!("盤面が不正です: {e}"))?;
    Ok((catalog, state))
}

fn to_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct KindInfo<'a> {
    id: &'a str,
    name: &'a str,
    /// 大きさ（横, 縦）。向きが北のとき。
    size: [u32; 2],
    attack: u32,
    hp: u32,
    castle: bool,
    immobile: bool,
    /// 成り先の種類の番号。
    promotes_to: Option<usize>,
}

pub fn piece_kinds() -> Result<String, String> {
    let catalog = catalog()?;
    let kinds: Vec<KindInfo> = catalog
        .iter()
        .map(|(kind, def)| KindInfo {
            id: &def.id,
            name: &def.name,
            size: def.size,
            attack: def.attack,
            hp: def.hp,
            castle: def.is_castle(),
            immobile: def.is_immobile(),
            promotes_to: catalog.promotion(kind).map(|k| k.0),
        })
        .collect();
    to_json(&kinds)
}

pub fn legal_actions(state: &str) -> Result<String, String> {
    let (catalog, state) = state_of(state)?;
    to_json(&rules_legal_actions(catalog, &state))
}

pub fn legal_targets(state: &str, piece: u32) -> Result<String, String> {
    let (catalog, state) = state_of(state)?;
    to_json(&rules_legal_targets(catalog, &state, PieceId(piece)))
}

pub fn legal_placements(state: &str, kind: u32) -> Result<String, String> {
    let (catalog, state) = state_of(state)?;
    let kind = KindId(kind as usize);
    if catalog.get(kind).is_none() {
        return Err(format!("不明な駒の種類です: {}", kind.0));
    }
    to_json(&rules_legal_placements(catalog, &state, kind))
}

#[derive(Serialize)]
struct Preview {
    events: Vec<rules::action::Event>,
    state: StateView,
}

pub fn preview(state: &str, action: &str) -> Result<String, String> {
    let (catalog, state) = state_of(state)?;
    let action: Action =
        serde_json::from_str(action).map_err(|e| format!("行動を読めません: {e}"))?;
    let (next, events) = apply(catalog, &state, &action).map_err(|e| e.to_string())?;
    to_json(&Preview {
        events,
        state: StateView::from_state(&next),
    })
}

pub fn threatened(state: &str) -> Result<String, String> {
    let (catalog, state) = state_of(state)?;
    to_json(&threatened_pieces(catalog, &state))
}
