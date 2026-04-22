#!/usr/bin/env bash
# 同步 package.json / tauri.conf.json / Cargo.toml 中的版本号
# 用法: ./scripts/version-bump.sh <version>
# 示例: ./scripts/version-bump.sh 0.2.0

set -euo pipefail

VERSION="${1:-}"

if [ -z "$VERSION" ]; then
    echo "用法: $0 <version>"
    echo "示例: $0 0.2.0"
    exit 1
fi

# 校验版本号格式 (semver)
if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
    echo "错误: 版本号格式不正确，应为 semver 格式 (例: 1.2.3 或 1.2.3-beta.1)"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "📦 更新版本号为 $VERSION ..."

# 1. package.json
sed -i.bak "s/\"version\": \".*\"/\"version\": \"$VERSION\"/" "$ROOT_DIR/package.json"
rm -f "$ROOT_DIR/package.json.bak"
echo "  ✅ package.json"

# 2. src-tauri/tauri.conf.json
sed -i.bak "s/\"version\": \".*\"/\"version\": \"$VERSION\"/" "$ROOT_DIR/src-tauri/tauri.conf.json"
rm -f "$ROOT_DIR/src-tauri/tauri.conf.json.bak"
echo "  ✅ src-tauri/tauri.conf.json"

# 3. src-tauri/Cargo.toml (仅替换 [package] 下的 version)
sed -i.bak "s/^version = \".*\"/version = \"$VERSION\"/" "$ROOT_DIR/src-tauri/Cargo.toml"
rm -f "$ROOT_DIR/src-tauri/Cargo.toml.bak"
echo "  ✅ src-tauri/Cargo.toml"

echo ""
echo "🎉 版本号已同步为 $VERSION"
echo ""
echo "后续步骤:"
echo "  git add -A"
echo "  git commit -m \"chore: bump version to $VERSION\""
echo "  git tag v$VERSION"
echo "  git push origin main --tags"
