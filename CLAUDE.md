# nandemo-shogi

## ドキュメント
- 仕様・設計は `docs/` にある。まず `docs/INDEX.md` で必要な資料を探す（本文は `@import` しない）。
- 仕様書の本体は `docs/specs/`。

## 開発フロー
- Issue 駆動。規則は `.claude/rules/issue-driven.md`、手順は `start-issue` / `commit-diff-html` / `create-pr` スキルを参照。

## コマンド
- テスト・静的解析: （未定。決まり次第ここに追記する）

## 規約
- シェルスクリプトの規約は `.claude/rules/shell.md`（`scripts/**/*.sh` 編集時のみ適用）。
- 個人設定は `CLAUDE.local.md` と `.claude/settings.local.json`（git 管理外）に置く。
