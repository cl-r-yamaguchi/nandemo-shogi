# main のブランチ保護

GitHub 側で `main` に設定している保護。安全フック（`scripts/hooks/block-dangerous.sh`）は正規表現のブロックリストで完全ではないため、サーバー側で止める最後の砦として設定している。

## 設定内容（2026-10-22 設定）

| 項目 | 値 | 意味 |
| --- | --- | --- |
| 強制 push（`allow_force_pushes`） | 禁止 | 履歴の書き換えを拒否する |
| ブランチ削除（`allow_deletions`） | 禁止 | `main` の削除を拒否する |
| PR 必須（`required_pull_request_reviews`） | 有効 | 直接 push を拒否し、PR 経由でのみ取り込む |
| 必要な承認数 | 0 | 個人開発のため。マージは人間が PR 上で行う |
| 管理者にも適用（`enforce_admins`） | 有効 | リポジトリ管理者でも上記を回避できない |

ステータスチェックは未設定（CI がないため）。

## 確認コマンド

```bash
gh api repos/cl-r-yamaguchi/nandemo-shogi/branches/main/protection
```

## 設定コマンド

```bash
gh api -X PUT repos/cl-r-yamaguchi/nandemo-shogi/branches/main/protection --input - <<'JSON'
{
  "required_status_checks": null,
  "enforce_admins": true,
  "required_pull_request_reviews": {
    "required_approving_review_count": 0,
    "dismiss_stale_reviews": false,
    "require_code_owner_reviews": false
  },
  "restrictions": null,
  "allow_force_pushes": false,
  "allow_deletions": false
}
JSON
```

## 注意

- 管理者にも適用しているため、緊急時でも `main` への直接 push はできない。必要なら保護を一時的に外す（GitHub の Settings → Branches、または API）。
- CI を導入したら、`required_status_checks` の設定を検討する。
