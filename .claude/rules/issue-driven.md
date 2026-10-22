# Issue駆動開発のルール

作業は必ず Issue 単位で進める。**1 Issue = 1 ブランチ = 1 PR**。
具体的なコマンドと手順は、`start-issue`（Issue・ブランチ）、`commit-diff-html`（コミット）、`create-pr`（PR）の各スキルを使う。

## 規則

- **Issue**: 着手前に必ず作る（既存 Issue があればそれを使う）。Issue なしで作業しない。
- **ブランチ**: `main` から、Issue 番号を含む名前で切る。例: `feature/123-短い説明`、`fix/123-短い説明`。1 つのブランチ／PR に複数 Issue を混ぜない。
- **作業日誌**: `docs/worklog/YYYY-MM-DD.md` に追記する。Issue 番号、やったこと、判断とその理由、未解決事項を残し、作業の区切りごとに更新する。
- **PR**: 本文に必ず `Closes #<Issue番号>` を入れる（マージ時に Issue が自動クローズされる）。
- **マージ**: 人間が行う。Claude は PR をマージせず、レビュー依頼までで止める。
- **push**: `main` へ直接 push しない。
