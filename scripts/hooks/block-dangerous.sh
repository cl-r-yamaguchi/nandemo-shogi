#!/usr/bin/env bash
# PreToolUse(Bash) フック: 危険なコマンドをブロックする（終了コード 2 でブロック）
# 判定できない場合（jq が無い、入力が不正など）も安全側に倒して終了コード 2 にする。
set -Eeuo pipefail
trap 'echo "ブロック: フックの処理に失敗したため、安全のため拒否します。" >&2; exit 2' ERR

if ! command -v jq >/dev/null 2>&1; then
  echo "ブロック: jq が見つからないため、コマンドを検査できません。" >&2
  exit 2
fi

# コマンド本体だけを取り出す（コマンド文字列内の危険な文字列は、実行しなくても検知される点に注意）
command="$(jq -r '.tool_input.command // ""')"

# リダイレクト（2>&1 など）の & で分割されないよう、先に取り除く
command="$(sed -E 's/[0-9]*>&[0-9-]?/ /g' <<<"$command")"

# ; & | で区切った各コマンド片ごとに判定する
while IFS= read -r segment; do
  # git push の強制: --force / --force-with-lease / -f（短い形式の連結を含む）/ +refspec
  # git のグローバルオプション（例: git -C dir push）を挟んでいても検知する
  # コマンド名の直前は空白に限らず、$( や バッククォート などの直後も境界として扱う
  if grep -Eq '(^|[^[:alnum:]_.-])git[[:space:]]([^[:space:]]+[[:space:]]+)*push([[:space:]]|$)' <<<"$segment" \
    && grep -Eq '[[:space:]](--force([[:space:]=)`]|$)|--force-with-lease|-[a-zA-Z]*f[a-zA-Z]*([[:space:])`]|$)|\+[^[:space:]])' <<<"$segment"; then
    echo "ブロック: 強制 push は禁止されています。" >&2
    exit 2
  fi

  # rm の再帰（-r / -R / --recursive）と強制（-f / --force）の組み合わせ（順序・分割指定を問わない）
  if grep -Eq '(^|[^[:alnum:]_.-])rm[[:space:]]' <<<"$segment" \
    && grep -Eq '[[:space:]](-[a-zA-Z]*[rR][a-zA-Z]*|--recursive)([[:space:]]|$)' <<<"$segment" \
    && grep -Eq '[[:space:]](-[a-zA-Z]*f[a-zA-Z]*|--force)([[:space:]]|$)' <<<"$segment"; then
    echo "ブロック: 再帰的な強制削除は禁止されています。" >&2
    exit 2
  fi
done < <(tr ';&|' '\n\n\n' <<<"$command")

exit 0
