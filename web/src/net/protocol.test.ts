import { describe, expect, it } from "vitest";
import type { ClientFrame, Event, ServerFrame } from "./index";
import { PROTOCOL_VERSION } from "./index";

// 生成された型が、サーバーとやり取りする JSON の形と合っていることを、型で確かめる。

describe("生成されたメッセージの型", () => {
  it("クライアントのメッセージを、型どおりに組み立てられる", () => {
    const act: ClientFrame = {
      v: PROTOCOL_VERSION,
      type: "act",
      turn: 3,
      action: { type: "move", piece: 4, to: { x: 1, y: 2 }, promote: false },
    };
    const pick: ClientFrame = {
      v: PROTOCOL_VERSION,
      type: "act",
      turn: 4,
      action: { type: "pick", index: 2 },
    };
    expect(JSON.parse(JSON.stringify(act))).toEqual(act);
    expect(pick.type).toBe("act");
  });

  it("サーバーのメッセージを、type で判別できる", () => {
    const describeFrame = (frame: ServerFrame): string => {
      switch (frame.type) {
        case "welcome":
          return `welcome:${frame.display_name}`;
        case "rejected":
          return `rejected:${frame.code}:${frame.reason ?? "-"}`;
        case "applied":
          return `applied:${frame.events.length}`;
        default:
          return frame.type;
      }
    };
    const rejected: ServerFrame = {
      v: PROTOCOL_VERSION,
      type: "rejected",
      code: "illegal_action",
      reason: "wrong_stage",
      message: "x",
    };
    expect(describeFrame(rejected)).toBe("rejected:illegal_action:wrong_stage");
  });

  it("出来事を、type で判別できる", () => {
    const damage = (event: Event): number => (event.type === "damaged" ? event.damage : 0);
    expect(damage({ type: "damaged", piece: 1, damage: 4, hp: 0 })).toBe(4);
    expect(damage({ type: "passed", player: 0 })).toBe(0);
  });
});
