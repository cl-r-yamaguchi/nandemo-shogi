---
paths:
  - "crates/**/*.rs"
  - "Cargo.toml"
  - "crates/**/Cargo.toml"
---

# Rust のルール

`crates/` 配下の Rust を編集するときだけ適用される。構成と責務は `docs/specs/architecture.md`。

- 整形は `cargo fmt`、静的解析は `cargo clippy -- -D warnings`。警告を残さない。
- `unsafe` は使わない（ワークスペースで禁止している）。
- `rules` は純粋な計算だけを置く。入出力・時刻・乱数の発生源に依存せず、乱数は引数で受け取る。
- 依存の向きは `rules` ← `ai`・`protocol`・`rules-wasm` ← `server`。逆向きの依存を足さない。
- 駒ごとの分岐を書かない。駒の違いは `data/pieces.toml` で表す。
- 通信メッセージの型は `protocol` にだけ書く。TypeScript の型は `cargo test -p protocol` で生成する（手で書かない）。
- 新しい規則は、`docs/specs/game-rules.md` の R- 番号を参照するテストと一緒に入れる。
