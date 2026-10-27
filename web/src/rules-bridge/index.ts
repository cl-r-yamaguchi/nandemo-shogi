// wasm の呼び出しの窓口。ほかのモジュールは、wasm を直接呼ばない。
// 状態は、サーバーが送る盤面の写し（StateView）。不正な入力は、例外（Error）になる。
import init, {
  board_size,
  legal_actions,
  legal_placements,
  legal_targets,
  piece_kinds,
  preview,
  threatened,
} from "./pkg/rules_wasm";
import type { Action, Event, StateView } from "../net";
import type { Facing } from "../net/generated/Facing";
import type { Pos } from "../net/generated/Pos";

/** 駒の種類の定義（`data/pieces.toml` から）。種類の番号は、配列の位置。 */
export interface PieceKind {
  id: string;
  name: string;
  /** 大きさ（横, 縦）。向きが北のとき。 */
  size: [number, number];
  attack: number;
  hp: number;
  castle: boolean;
  immobile: boolean;
  /** 成り先の種類の番号。 */
  promotes_to: number | null;
}

export interface Placement {
  anchor: Pos;
  facing: Facing;
}

export interface Preview {
  events: Event[];
  /** 行動のあとの状態。脱落の順位は近似（読み取り用）。 */
  state: StateView;
}

export interface Rules {
  boardSize: number;
  kinds: PieceKind[];
  /** 手番のプレイヤーの、合法な行動。 */
  legalActions(state: StateView): Action[];
  /** 盤上の駒の、移動先のアンカー。 */
  legalTargets(state: StateView, piece: number): Pos[];
  /** 種類 `kind` の駒を置ける、アンカーと向き。 */
  legalPlacements(state: StateView, kind: number): Placement[];
  /** 行動を適用した結果の、プレビュー。 */
  preview(state: StateView, action: Action): Preview;
  /** 捕食される危険のある駒の番号。 */
  threatened(state: StateView): number[];
}

/** wasm の関数を呼んで、JSON の結果を読む。失敗（JsError）は、Error の例外として届く。 */
function call<T>(fn: () => string): T {
  return JSON.parse(fn()) as T;
}

/** wasm を読み込む。`wasm` は、テストなどで、バイト列を直接渡すとき。 */
export async function loadRules(wasm?: BufferSource): Promise<Rules> {
  await (wasm === undefined ? init() : init({ module_or_path: wasm }));
  return {
    boardSize: board_size(),
    kinds: call<PieceKind[]>(piece_kinds),
    legalActions: (state) => call(() => legal_actions(JSON.stringify(state))),
    legalTargets: (state, piece) =>
      call(() => legal_targets(JSON.stringify(state), piece)),
    legalPlacements: (state, kind) => {
      const raw = call<[Pos, Facing][]>(() =>
        legal_placements(JSON.stringify(state), kind),
      );
      return raw.map(([anchor, facing]) => ({ anchor, facing }));
    },
    preview: (state, action) =>
      call(() => preview(JSON.stringify(state), JSON.stringify(action))),
    threatened: (state) => call(() => threatened(JSON.stringify(state))),
  };
}
