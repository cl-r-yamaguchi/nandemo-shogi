---
name: create-pr
description: 現在のブランチから Pull Request を作成する。本文に Closes #Issue番号 を入れてマージ時に Issue を自動クローズする。作業が完了して PR を出すときに使う。
---

# PR の作成

## 手順

1. `git status` と `git diff main...HEAD` で変更内容を確認する。未コミットがあれば先に `commit-diff-html` スキルでコミットする。
2. **自己レビュー**: `/code-review` を実行し、指摘を直す。直した変更は `commit-diff-html` スキルでコミットする。
3. **main の取り込み**: main が先に進んでいれば取り込む。競合は Claude が解消し、解消後の `git diff` を人が確認する。
   ```bash
   git fetch origin main
   git merge origin/main
   ```
4. ブランチ名から Issue 番号を特定する。特定できなければユーザーに聞く。
5. push する（確認が入るのでユーザーの承認を待つ）。
   ```bash
   git push -u origin HEAD
   ```
6. PR を作成する。本文に必ず `Closes #<番号>` を入れる。
   ```bash
   gh pr create --title "<タイトル>" --body "$(cat <<'BODY'
   ## 概要
   <何を・なぜ変更したか>

   ## 変更内容
   - <変更点>

   ## 確認方法
   - <確認手順>

   Closes #<番号>
   BODY
   )"
   ```
7. PR の URL を報告する。マージはしない（`issue-driven.md` の規則）。
