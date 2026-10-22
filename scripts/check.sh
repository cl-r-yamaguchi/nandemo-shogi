#!/usr/bin/env bash
# 全体の検査: Rust（整形・静的解析・テスト）とフロントエンド（型・lint・テスト）。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# Homebrew 版の rustup は PATH に入らないことがあるため、あれば足す
if ! command -v cargo >/dev/null 2>&1 && [ -d /opt/homebrew/opt/rustup/bin ]; then
  export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
fi

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

for tool in wasm-pack npm; do
  command -v "$tool" >/dev/null 2>&1 || { echo "$tool が見つかりません（CLAUDE.md の前提を参照）" >&2; exit 1; }
done

cd web
[ -d node_modules ] || npm ci
npm run wasm
npm run typecheck
npm run lint
npm run test
