---
paths:
  - "web/src/**/*.ts"
  - "web/src/**/*.css"
---

# TypeScript・フロントエンドのルール

`web/src/` を編集するときだけ適用される。構成と責務は `docs/specs/architecture.md`、画面は `docs/specs/ui.md`。

- 型チェック（`strict`）と `eslint` を通す。`any` を使わない。
- 依存の向きは `ui` → `store` → `net`、`ui`・`store` → `rules-bridge`、`scene` → `store`（読み取りのみ）。逆向きや横断をしない。
- wasm は `rules-bridge` からだけ呼ぶ。
- `src/net/generated/` と `src/rules-bridge/pkg/` は生成物。手で編集しない。
- 色・大きさ・動きの値は `src/tokens/` にだけ書き、部品はそれを参照する。値を直書きしない。
- OS・ブラウザ標準の見た目の部品（`<select>`、`alert`、`confirm` など）を使わない。UI の値の変化は、必ずアニメーションでつなぐ。
