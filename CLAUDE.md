# nandemo-shogi

## ドキュメント
- 仕様・設計は `docs/` にある。まず `docs/INDEX.md` で必要な資料を探す（本文は `@import` しない）。
- 仕様書の本体は `docs/specs/`。

## 開発フロー
- Issue 駆動。規則は `.claude/rules/issue-driven.md`、手順は `start-issue` / `commit-diff-html` / `create-pr` スキルを参照。

## コマンド
- 前提: Rust（`rustup`）、`wasm-pack`、Node.js・npm。Homebrew 版の `rustup` は、`export PATH="/opt/homebrew/opt/rustup/bin:$PATH"` が必要なことがある。
- 全体の検査（整形・静的解析・テスト・型・lint）: `scripts/check.sh`
- Rust: `cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`
- TypeScript の型生成: `cargo test -p protocol`（`web/src/net/generated/` に出力。手で編集しない）
- フロントエンド（`web/` で実行）: `npm install`、`npm run dev`（wasm のビルド込みで、`127.0.0.1:5173` に起動）、`npm run build`、`npm run typecheck`、`npm run lint`、`npm test`
- wasm のビルド（`web/` で実行）: `npm run wasm`（出力は `web/src/rules-bridge/pkg/`。git 管理外）
- サーバーの設定: 環境変数 `NANDEMO_BIND`・`NANDEMO_ALLOWED_ORIGINS`・`NANDEMO_DB_PATH`（初期値はローカルホストのみ）

## 規約
- シェルスクリプトの規約は `.claude/rules/shell.md`（`scripts/**/*.sh` 編集時のみ適用）。
- Rust は `.claude/rules/rust.md`（`crates/**/*.rs` 編集時のみ適用）、TypeScript は `.claude/rules/typescript.md`（`web/src/**` 編集時のみ適用）。
- プロジェクト内のファイル（docs・コード・コメント・コミット・PR・Issue）に、チャットでのやり取りの文脈（「ユーザーの指示で」「対話で確認済み」「貼られたメモ」など）を書かない。成果物は、それ単体で読めるようにする。
- 個人設定は `CLAUDE.local.md` と `.claude/settings.local.json`（git 管理外）に置く。
