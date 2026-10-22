#!/usr/bin/env bash
# PreToolUse(Bash) フック: 危険なコマンドをブロックする（終了コード 2 でブロック）
set -euo pipefail

# コマンド本体だけを判定する（heredoc 内のサンプル文字列等はコマンド全体に含まれるため、引用符内は除外しきれない点に注意）
command="$(jq -r '.tool_input.command // ""')"

if grep -Eq '(^|[;&|[:space:]])git[[:space:]]+push([[:space:]]+[^;&|]*)?[[:space:]]+(--force([[:space:]]|$)|-[a-zA-Z]*f[a-zA-Z]*([[:space:]]|$))' <<<"$command"; then
  echo "ブロック: git push --force は禁止されています。" >&2
  exit 2
fi

if grep -Eq '(^|[;&|[:space:]])rm[[:space:]]+(-[a-zA-Z]*r[a-zA-Z]*f|-[a-zA-Z]*f[a-zA-Z]*r|-r[[:space:]]+-f|-f[[:space:]]+-r)' <<<"$command"; then
  echo "ブロック: rm -rf は禁止されています。" >&2
  exit 2
fi

exit 0
