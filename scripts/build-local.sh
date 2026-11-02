#!/bin/bash
#
# 本地构建脚本
# 支持构建中文和英文镜像
#

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

print_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
print_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
print_warning() { echo -e "${YELLOW}[WARNING]${NC} $1"; }
print_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# 配置
VERSION="${1:-v1.0.0}"
# Dockerfile_zh/Dockerfile_en were merged into one Dockerfile (63ccff5); the
# mirror choice is now a build argument. Build from the repository root.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
REGISTRY_CN="registry.cn-hangzhou.aliyuncs.com/fliaping/agent-workspace"
REGISTRY_EN="xuping/agent-workspace"

echo "========================================"
echo "  Cranibox 本地构建"
echo "========================================"
echo ""

# 选择构建类型
echo "请选择要构建的镜像类型："
echo "  1) 中文镜像 (使用阿里云源) - 推送到阿里云"
echo "  2) 英文镜像 (使用官方源) - 推送到 Docker Hub"
echo "  3) 同时构建两个镜像"
echo ""
read -p "请输入选项 [1-3]: " BUILD_CHOICE

case $BUILD_CHOICE in
    1)
        BUILD_CN=true
        BUILD_EN=false
        ;;
    2)
        BUILD_CN=false
        BUILD_EN=true
        ;;
    3)
        BUILD_CN=true
        BUILD_EN=true
        ;;
    *)
        print_error "无效选项"
        exit 1
        ;;
esac

# 构建中文镜像
build_chinese() {
    echo ""
    print_info "构建中文镜像..."
    print_info "Dockerfile: Dockerfile (USE_CHINA_MIRROR=true)"
    print_info "版本: $VERSION"
    
    docker build -t "agent-workspace-zh:${VERSION}" -f Dockerfile \
        --build-arg USE_CHINA_MIRROR=true "$REPO_ROOT"
    
    # 打标签
    docker tag "agent-workspace-zh:${VERSION}" "${REGISTRY_CN}:${VERSION}"
    
    print_success "中文镜像构建完成"
    print_info "本地标签: agent-workspace-zh:${VERSION}"
    print_info "阿里云标签: ${REGISTRY_CN}:${VERSION}"
    
    # 询问是否推送
    read -p "是否推送到阿里云仓库? [y/N]: " PUSH_CN
    if [[ "$PUSH_CN" =~ ^[Yy]$ ]]; then
        print_info "推送到阿里云..."
        docker push "${REGISTRY_CN}:${VERSION}"
        print_success "推送完成"
    fi
}

# 构建英文镜像
build_english() {
    echo ""
    print_info "构建英文镜像..."
    print_info "Dockerfile: Dockerfile (USE_CHINA_MIRROR=false)"
    print_info "版本: $VERSION"
    
    docker build -t "agent-workspace-en:${VERSION}" -f Dockerfile \
        --build-arg USE_CHINA_MIRROR=false "$REPO_ROOT"
    
    # 打标签
    docker tag "agent-workspace-en:${VERSION}" "${REGISTRY_EN}:${VERSION}"
    
    print_success "英文镜像构建完成"
    print_info "本地标签: agent-workspace-en:${VERSION}"
    print_info "Docker Hub标签: ${REGISTRY_EN}:${VERSION}"
    
    # 询问是否推送
    read -p "是否推送到 Docker Hub? [y/N]: " PUSH_EN
    if [[ "$PUSH_EN" =~ ^[Yy]$ ]]; then
        print_info "推送到 Docker Hub..."
        docker push "${REGISTRY_EN}:${VERSION}"
        print_success "推送完成"
    fi
}

# 执行构建
if [ "$BUILD_CN" = true ]; then
    build_chinese
fi

if [ "$BUILD_EN" = true ]; then
    build_english
fi

echo ""
echo "========================================"
print_success "构建流程完成!"
echo "========================================"
echo ""

# 显示本地镜像
docker images | grep "agent-workspace" || true
echo ""
