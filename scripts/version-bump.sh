#!/usr/bin/env bash
# 同步 package.json / package-lock.json / tauri.conf.json / Cargo.toml / Cargo.lock 中的版本号
# 用法: ./scripts/version-bump.sh [version]
# 示例: ./scripts/version-bump.sh 0.2.0
# 不传 version 时，自动递增 patch 版本

set -euo pipefail

VERSION_INPUT="${1:-}"

if [ "$#" -gt 1 ]; then
    echo "用法: $0 [version]"
    echo "示例: $0 0.2.0"
    echo "不传 version 时，自动递增 patch 版本"
    exit 1
fi

# 校验手动版本号格式 (semver)
if [ -n "$VERSION_INPUT" ] && ! echo "$VERSION_INPUT" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
    echo "错误: 版本号格式不正确，应为 semver 格式 (例: 1.2.3 或 1.2.3-beta.1)"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
VERSION_TARGET="${VERSION_INPUT:-patch}"

if [ -z "$VERSION_INPUT" ]; then
    echo "📦 未指定版本号，自动递增 patch 版本 ..."
else
    echo "📦 更新版本号为 $VERSION_INPUT ..."
fi

# 1. package.json / package-lock.json
NPM_VERSION_OUTPUT="$(
    cd "$ROOT_DIR"
    npm version "$VERSION_TARGET" --no-git-tag-version --ignore-scripts --allow-same-version
)"
VERSION="$(printf '%s\n' "$NPM_VERSION_OUTPUT" | tail -n 1 | sed 's/^v//')"
echo "  ✅ package.json"
echo "  ✅ package-lock.json"

# 2. src-tauri/tauri.conf.json
sed -i.bak "s/\"version\": \".*\"/\"version\": \"$VERSION\"/" "$ROOT_DIR/src-tauri/tauri.conf.json"
rm -f "$ROOT_DIR/src-tauri/tauri.conf.json.bak"
echo "  ✅ src-tauri/tauri.conf.json"

# 3. src-tauri/Cargo.toml (仅替换 [package] 下的 version)
sed -i.bak "s/^version = \".*\"/version = \"$VERSION\"/" "$ROOT_DIR/src-tauri/Cargo.toml"
rm -f "$ROOT_DIR/src-tauri/Cargo.toml.bak"
echo "  ✅ src-tauri/Cargo.toml"

# 4. src-tauri/Cargo.lock (仅替换 token-burger 包条目)
VERSION="$VERSION" perl -0pi.bak -e 's/(\[\[package\]\]\nname = "token-burger"\nversion = ")[^"]+(")/$1$ENV{VERSION}$2/' "$ROOT_DIR/src-tauri/Cargo.lock"
rm -f "$ROOT_DIR/src-tauri/Cargo.lock.bak"

if ! awk -v version="$VERSION" '
    previous == "name = \"token-burger\"" && $0 == "version = \"" version "\"" {
        found = 1
    }
    {
        previous = $0
    }
    END {
        exit found ? 0 : 1
    }
' "$ROOT_DIR/src-tauri/Cargo.lock"; then
    echo "错误: 未能更新 src-tauri/Cargo.lock 中 token-burger 的版本号"
    exit 1
fi
echo "  ✅ src-tauri/Cargo.lock"

echo ""
echo "🎉 版本号已同步为 $VERSION"
echo ""
CONFIRM=""
read -r -p "是否自动暂存指定版本文件并提交？输入 yes 确认: " CONFIRM || true

if [ "$CONFIRM" = "yes" ]; then
    VERSION_FILES=(
        "$ROOT_DIR/package.json"
        "$ROOT_DIR/package-lock.json"
        "$ROOT_DIR/src-tauri/tauri.conf.json"
        "$ROOT_DIR/src-tauri/Cargo.toml"
        "$ROOT_DIR/src-tauri/Cargo.lock"
    )

    if git -C "$ROOT_DIR" show-ref --verify --quiet "refs/tags/v$VERSION"; then
        echo "错误: tag v$VERSION 已存在"
        exit 1
    fi

    git -C "$ROOT_DIR" add -- "${VERSION_FILES[@]}"

    if git -C "$ROOT_DIR" diff --cached --quiet -- "${VERSION_FILES[@]}"; then
        echo "没有可提交的版本号变更"
    else
        git -C "$ROOT_DIR" commit -m "chore: bump version to $VERSION" -- "${VERSION_FILES[@]}"
        git -C "$ROOT_DIR" tag "v$VERSION"
        echo "✅ 已提交版本号变更"
        echo "✅ 已创建 tag v$VERSION"
    fi
else
    echo "已跳过 git add / commit / tag"
fi
