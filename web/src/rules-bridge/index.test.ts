/// <reference types="node" />
import { readFileSync } from "node:fs";
import { beforeAll, describe, expect, it } from "vitest";
import type { StateView } from "../net";
import { loadRules, type Rules } from "./index";

let rules: Rules;

beforeAll(async () => {
  const bytes = readFileSync(new URL("./pkg/rules_wasm_bg.wasm", import.meta.url));
  rules = await loadRules(bytes);
});

/** 配置フェーズの最初の状態（3 人。盤上に駒はなく、各自、城を 1 つ持つ）。 */
function fresh(castle: number): StateView {
  return {
    board_size: 16,
    phase: "placement",
    stage: { type: "act" },
    current: 0,
    turn: 0,
    players: [0, 1, 2].map(() => ({
      eliminated: false,
      hand: [{ kind: castle, count: 1 }],
    })),
    pieces: [],
    outcome: null,
  };
}

function castleKind(): number {
  return rules.kinds.findIndex((k) => k.castle);
}

describe("rules-bridge", () => {
  it("盤の大きさと、駒の種類を返す", () => {
    expect(rules.boardSize).toBe(16);
    const castle = rules.kinds[castleKind()];
    expect(castle).toMatchObject({ castle: true, immobile: true, size: [2, 2] });
  });

  it("配置フェーズでは、城を盤内に置ける場所が返る", () => {
    const state = fresh(castleKind());
    const placements = rules.legalPlacements(state, castleKind());
    expect(placements.length).toBeGreaterThan(0);
    for (const { anchor } of placements) {
      expect(anchor.x).toBeGreaterThanOrEqual(0);
      expect(anchor.x + 2).toBeLessThanOrEqual(16);
    }
  });

  it("行動のプレビューは、置いた駒を含む状態を返す", () => {
    const state = fresh(castleKind());
    const first = rules.legalPlacements(state, castleKind())[0];
    expect(first).toBeDefined();
    const result = rules.preview(state, {
      type: "place",
      kind: castleKind(),
      anchor: first!.anchor,
      facing: first!.facing,
    });
    expect(result.state.pieces).toHaveLength(1);
    expect(result.events.some((e) => e.type === "placed")).toBe(true);
    expect(rules.threatened(result.state)).toEqual([]);
  });

  it("不正な入力は、例外になる", () => {
    const state = fresh(castleKind());
    expect(() => rules.legalActions({ ...state, board_size: 0 })).toThrow();
    expect(() => rules.legalPlacements(state, 9999)).toThrow();
    expect(() => rules.preview(state, { type: "pass" })).toThrow();
  });
});
