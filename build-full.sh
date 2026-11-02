#!/bin/bash
# Cranibox 本地构建脚本

set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

DESKTOP="${DESKTOP:-lxqt}"
IMAGE_NAME="xuping/agent-workspace"
IMAGE_TAG="ubuntu-${DESKTOP}"
EXPORT_FILE="agent-workspace-${DESKTOP}.tar"

echo "======================================"
echo "  Cranibox 构建脚本"
echo "  桌面: ${DESKTOP} (lxqt/xfce/kde)"
echo "======================================"
echo ""

# 检查 Dockerfile
if [ ! -f "Dockerfile" ]; then
    echo "❌ 错误: Dockerfile 不存在"
    exit 1
fi

echo "步骤 1: 构建镜像"
echo "=================="

docker build --pull --build-arg DESKTOP=${DESKTOP} -t ${IMAGE_NAME}:${IMAGE_TAG} . 2>&1 | tee build.log

echo ""
echo "✅ 镜像构建成功"
docker images ${IMAGE_NAME}:${IMAGE_TAG}

echo ""
echo "步骤 2: 导出镜像"
echo "=================="
echo "导出中，这可能需要几分钟..."
docker save -o ${EXPORT_FILE} ${IMAGE_NAME}:${IMAGE_TAG}

echo ""
echo "✅ 镜像已导出: ${EXPORT_FILE}"
ls -lh ${EXPORT_FILE}

echo ""
echo "======================================"
echo "  构建完成!"
echo "======================================"
echo ""
echo "镜像: ${IMAGE_NAME}:${IMAGE_TAG}"
echo "文件: ${EXPORT_FILE} ($(du -h ${EXPORT_FILE} | cut -f1))"
echo ""
echo "使用方法:"
echo "  docker load -i ${EXPORT_FILE}"
echo "  docker compose up -d"
echo ""
echo "构建其他桌面:"
echo "  DESKTOP=kde  ./build-full.sh"
echo "  DESKTOP=xfce ./build-full.sh"
echo ""
