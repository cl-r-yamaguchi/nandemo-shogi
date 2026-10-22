import "./tokens/index.css";
import { loadRules } from "./rules-bridge";

const app = document.querySelector<HTMLDivElement>("#app");
if (!app) throw new Error("#app が見つかりません");

loadRules()
  .then((rules) => {
    app.textContent = `盤は ${rules.boardSize}x${rules.boardSize} マス（wasm から取得）`;
  })
  .catch((error: unknown) => {
    app.textContent = "ルールの読み込みに失敗しました。`npm run wasm` を実行してください。";
    console.error(error);
  });
