// wasm の呼び出しの窓口。ほかのモジュールは、wasm を直接呼ばない。
import init, { board_size } from "./pkg/rules_wasm";

export interface Rules {
  boardSize: number;
}

export async function loadRules(): Promise<Rules> {
  await init();
  return { boardSize: board_size() };
}
