# docs インデックス

`docs/` 配下のドキュメント一覧。ドキュメントを追加・削除したらこの表も更新すること。

| パス | 説明 | 読むタイミング |
| --- | --- | --- |
| [docs/specs/spec.md](specs/spec.md) | 仕様書の入口。目的、ゲームの概要、用語、対象外、仕様書一式の案内 | 最初に読む。仕様の全体像、用語を知りたいとき |
| [docs/specs/game-rules.md](specs/game-rules.md) | ゲームルールの完全な定義（R- 番号つき。進行・行動・戦闘・成り・勝敗） | ルール・AI の実装、テストを書く、挙動の正解を確認するとき |
| [docs/specs/pieces.md](specs/pieces.md) | 駒のデータ（大きさ・攻撃力・HP・移動範囲・出現重み・成り先）と移動範囲の表現 | 駒を追加・調整するとき、駒の強さや動きを確認するとき |
| [docs/specs/ui.md](specs/ui.md) | 画面、操作、3D 表示、自作 UI 部品、アニメーション、デザイントークン | フロントエンド・UI を実装するとき |
| [docs/specs/online.md](specs/online.md) | ルーム、マッチング、席順、再接続、通信プロトコル、AI、対局記録 | サーバー・通信・AI を実装するとき |
| [docs/specs/architecture.md](specs/architecture.md) | モジュール構成、依存の向き、データの所有者、技術選定、非機能要件 | 実装を始める前、モジュールの置き場所や責務に迷ったとき |
| [docs/ops/branch-protection.md](ops/branch-protection.md) | GitHub の `main` のブランチ保護の設定内容と確認・設定コマンド | 保護設定を確認・変更するとき、`main` へ push できない理由を知りたいとき |
| [docs/worklog/](worklog/) | 日付ごとの作業日誌（Issue 番号、やったこと、判断の理由、未解決事項） | 過去の経緯や判断の理由を知りたいとき、作業を再開するとき |
