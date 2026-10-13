#!/bin/bash
#
# Agent Workspace 一键部署脚本
# 基于 LinuxServer webtop (Selkies)
# 支持多语言: 中文 / English
# 使用方法: curl -fsSL https://raw.githubusercontent.com/fliaping/cranibox/main/install.sh | sudo bash
#

set -e

# ============================================================================
# 终端输入处理（支持 curl | bash 管道模式）
# ============================================================================
# 当通过 curl | bash 执行时，stdin 被管道占用，read 无法从终端读取
# 解决方案：打开 /dev/tty 作为 fd 3，所有交互式 read 从 fd 3 读取
if [ -t 0 ]; then
    # 直接执行，stdin 就是终端
    exec 3<&0
else
    # 管道模式，从 /dev/tty 打开终端
    exec 3</dev/tty
fi

# 封装 read 函数，自动从 fd 3（终端）读取
prompt_read() {
    local varname="$1"
    shift
    read -p "$@" "$varname" <&3
}

# ============================================================================
# 多语言配置
# ============================================================================

# 多语言文本字典

# 中文文本
TEXT_cn_welcome_title="Agent Workspace 一键部署"
TEXT_cn_step1_title="步骤 1/9: 选择语言"
TEXT_cn_lang_cn="1) 中文 (Chinese)"
TEXT_cn_lang_en="2) English (英文)"
TEXT_cn_step_desktop_title="步骤 2/9: 选择桌面环境"
TEXT_cn_desktop_lxqt="2) LXQt  — 轻量 (~300MB 内存)"
TEXT_cn_desktop_xfce="1) XFCE  — 推荐 (~800MB 内存)"
TEXT_cn_desktop_kde="3) KDE   — 完整 (~1.1GB 内存)"
TEXT_cn_selected_desktop="已选择桌面"
TEXT_cn_step_registry_title="步骤 4/9: 选择镜像源"
TEXT_cn_registry_cn="1) 阿里云镜像（国内推荐）"
TEXT_cn_registry_en="2) Docker Hub（海外推荐）"
TEXT_cn_enter_choice="请输入选项"
TEXT_cn_invalid_choice="无效选项，请重新输入"
TEXT_cn_step2_title="步骤 5/9: 选择镜像版本"
TEXT_cn_version_latest="1) latest (最新版)"
TEXT_cn_version_custom="2) 自定义版本"
TEXT_cn_enter_version="请输入版本号"
TEXT_cn_step3_title="步骤 6/9: 配置数据目录"
TEXT_cn_data_dir_default="使用默认目录"
TEXT_cn_data_dir_custom="自定义目录"
TEXT_cn_enter_data_dir="请输入数据目录路径"
TEXT_cn_step4_title="步骤 7/9: 配置桌面端口"
TEXT_cn_port_config_title="配置服务端口"
TEXT_cn_port_desktop="桌面端口 (HTTPS)"
TEXT_cn_port_agent="Agent API 端口"
TEXT_cn_port_auto="自动检测并分配端口"
TEXT_cn_port_manual="手动配置端口"
TEXT_cn_enter_desktop_port="请输入桌面端口"
TEXT_cn_enter_agent_port="请输入 Agent 端口"
TEXT_cn_port_in_use="端口已被占用"
TEXT_cn_port_available="端口可用"
TEXT_cn_step5_title="步骤 8/9: 选择 Agent 软件"
TEXT_cn_agent_install_title="选择要安装的 Agent 软件（可多选，空格分隔）"
TEXT_cn_agent_codex="1) Codex - OpenAI 编程 Agent（推荐，安装后运行 codex 登录）"
TEXT_cn_agent_claude_code="2) Claude Code - Anthropic 编程 Agent（安装后运行 claude 登录）"
TEXT_cn_agent_hermes="3) Hermes Agent - 通用自主 Agent（安装后运行 hermes setup --portal）"
TEXT_cn_agent_openclaw="4) OpenClaw - 个人自主开源 AI 助手，支持多平台通信"
TEXT_cn_agent_openfang="5) Openfang - Rust 构建的 Agent OS"
TEXT_cn_agent_zeroclaw="6) Zeroclaw - 超轻量 Agent 运行时"
TEXT_cn_agent_skip="7) 跳过，不安装任何软件"
TEXT_cn_enter_agents="请输入选项（如：1 2 3）"
TEXT_cn_step6_title="步骤 9/9: 配置 Agent 软件端口"
TEXT_cn_agent_port_config="配置 Agent 软件端口"
TEXT_cn_agent_port_default="使用默认端口"
TEXT_cn_agent_port_custom="自定义端口"
TEXT_cn_enter_agent_port_num="请输入端口"
TEXT_cn_agent_port_openclaw="OpenClaw 端口（默认 18789）"
TEXT_cn_agent_port_openfang="Openfang 端口（默认 4200）"
TEXT_cn_agent_port_zeroclaw="Zeroclaw 端口（默认 42617）"
TEXT_cn_installing_agents="正在安装 Agent 软件..."
TEXT_cn_agent_install_success="Agent 软件安装成功"
TEXT_cn_using_registry="使用镜像仓库"
TEXT_cn_selected_image="已选择镜像"
TEXT_cn_pulling="正在拉取镜像..."
TEXT_cn_pull_success="镜像拉取成功"
TEXT_cn_pull_failed="镜像拉取失败"
TEXT_cn_running="正在启动容器..."
TEXT_cn_run_success="容器启动成功"
TEXT_cn_run_failed="容器启动失败"
TEXT_cn_complete="部署完成"
TEXT_cn_access_info="访问信息"
TEXT_cn_desktop_url="桌面"
TEXT_cn_agent_port="Agent端口"
TEXT_cn_data_dir="数据目录"
TEXT_cn_common_commands="常用命令"
TEXT_cn_view_logs="查看日志"
TEXT_cn_stop_service="停止服务"
TEXT_cn_start_service="启动服务"
TEXT_cn_enter_container="进入容器"
TEXT_cn_check_docker="检查 Docker 环境..."
TEXT_cn_docker_not_installed="Docker 未安装"
TEXT_cn_docker_install_guide="请安装 Docker"
TEXT_cn_docker_daemon_not_running="Docker 守护进程未运行"
TEXT_cn_start_docker_service="请启动 Docker 服务"
TEXT_cn_docker_compose_not_installed="Docker Compose 未安装"
TEXT_cn_container_exists="容器已存在"
TEXT_cn_delete_recreate="是否删除并重新创建?"
TEXT_cn_deleting_old="正在删除旧容器..."
TEXT_cn_starting_existing="正在启动已有容器..."
TEXT_cn_port_occupied="端口被占用"
TEXT_cn_auto_change_port="自动更换端口为"
TEXT_cn_creating_data_dir="正在创建数据目录..."
TEXT_cn_waiting_service="等待服务启动..."
TEXT_cn_os_detected="检测到操作系统"
TEXT_cn_dind_detected="检测到 Docker in Docker (DinD) 环境"
TEXT_cn_network_mode_title="DinD 环境网络配置"
TEXT_cn_network_host="1) host 模式 (推荐，直接使用宿主机网络)"
TEXT_cn_network_bridge="2) bridge 模式 + 主机名映射"
TEXT_cn_select_network="请选择网络模式"
TEXT_cn_using_host_network="使用 host 网络模式"
TEXT_cn_using_bridge_network="使用 bridge 网络模式"
TEXT_cn_need_sudo="Linux 需要 root 权限，请使用 sudo 运行"
TEXT_cn_usage="使用方法"
TEXT_cn_dind_sudo_warning="DinD 环境中使用 sudo 可能导致 Docker 连接失败"
TEXT_cn_try_without_sudo="请尝试不使用 sudo 运行"
TEXT_cn_gpu_detecting="检测 GPU 加速支持..."
TEXT_cn_gpu_nvidia="检测到 NVIDIA GPU"
TEXT_cn_gpu_nvidia_runtime="NVIDIA Container Runtime 可用"
TEXT_cn_gpu_nvidia_no_runtime="未检测到 NVIDIA Container Runtime，跳过 GPU 加速"
TEXT_cn_gpu_nvidia_toolkit_guide="请安装 nvidia-container-toolkit: https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/install-guide.html"
TEXT_cn_gpu_intel_amd="检测到 Intel/AMD GPU (DRI)"
TEXT_cn_gpu_none="未检测到 GPU，使用 CPU 软件渲染"
TEXT_cn_gpu_enabled="GPU 加速已启用"
TEXT_cn_step_docker_title="步骤 3/9: Docker 配置"
TEXT_cn_docker_mode_none="1) 不启用 Docker（默认）"
TEXT_cn_docker_mode_dind="2) DinD 模式（容器内独立 Docker，需要 --privileged）"
TEXT_cn_docker_mode_socket="3) 挂载宿主机 Docker（共享宿主机 Docker 守护进程）"
TEXT_cn_docker_mode_selected="Docker 模式"
TEXT_cn_docker_mode_none_desc="不启用"
TEXT_cn_docker_mode_dind_desc="DinD（独立 Docker）"
TEXT_cn_docker_mode_socket_desc="挂载宿主机 Docker"
TEXT_cn_ssh_port="SSH 端口"
TEXT_cn_ssh_password_prompt="设置 SSH 密码（留空则不启用 SSH）"

# 英文文本
TEXT_en_welcome_title="Agent Workspace Deployment"
TEXT_en_step1_title="Step 1/9: Select Language"
TEXT_en_lang_cn="1) 中文 (Chinese)"
TEXT_en_lang_en="2) English"
TEXT_en_step_desktop_title="Step 2/9: Select Desktop"
TEXT_en_desktop_lxqt="2) LXQt  — Lightweight (~300MB RAM)"
TEXT_en_desktop_xfce="1) XFCE  — Recommended (~800MB RAM)"
TEXT_en_desktop_kde="3) KDE   — Full (~1.1GB RAM)"
TEXT_en_selected_desktop="Selected desktop"
TEXT_en_enter_choice="Enter your choice"
TEXT_en_invalid_choice="Invalid choice, please try again"
TEXT_en_step_registry_title="Step 4/9: Select Registry"
TEXT_en_registry_cn="1) Alibaba Cloud (Recommended for China)"
TEXT_en_registry_en="2) Docker Hub (Recommended overseas)"
TEXT_en_step2_title="Step 5/9: Select Version"
TEXT_en_version_latest="1) latest"
TEXT_en_version_custom="2) Custom version"
TEXT_en_enter_version="Enter version tag"
TEXT_en_step3_title="Step 6/9: Configure Data Directory"
TEXT_en_data_dir_default="Use default directory"
TEXT_en_data_dir_custom="Custom directory"
TEXT_en_step4_title="Step 7/9: Configure Desktop Port"
TEXT_en_port_config_title="Configure Service Ports"
TEXT_en_port_desktop="Desktop Port (HTTPS)"
TEXT_en_port_agent="Agent API Port"
TEXT_en_port_auto="Auto detect and assign ports"
TEXT_en_port_manual="Manually configure ports"
TEXT_en_enter_desktop_port="Enter desktop port"
TEXT_en_enter_agent_port="Enter Agent port"
TEXT_en_port_in_use="Port is in use"
TEXT_en_port_available="Port is available"
TEXT_en_enter_data_dir="Enter data directory path"
TEXT_en_step5_title="Step 8/9: Select Agent Software"
TEXT_en_agent_install_title="Select Agent software to install (multiple choices allowed, space separated)"
TEXT_en_agent_codex="1) Codex - OpenAI coding agent (recommended; run codex to sign in)"
TEXT_en_agent_claude_code="2) Claude Code - Anthropic coding agent (run claude to sign in)"
TEXT_en_agent_hermes="3) Hermes Agent - General autonomous agent (run hermes setup --portal)"
TEXT_en_agent_openclaw="4) OpenClaw - Personal autonomous AI assistant"
TEXT_en_agent_openfang="5) Openfang - Rust Agent OS"
TEXT_en_agent_zeroclaw="6) Zeroclaw - Ultra-light Agent runtime"
TEXT_en_agent_skip="7) Skip, don't install any software"
TEXT_en_enter_agents="Enter options (e.g., 1 2 3)"
TEXT_en_step6_title="Step 9/9: Configure Agent Software Ports"
TEXT_en_agent_port_config="Configure Agent Software Ports"
TEXT_en_agent_port_default="Use default port"
TEXT_en_agent_port_custom="Custom port"
TEXT_en_enter_agent_port_num="Enter port"
TEXT_en_agent_port_openclaw="OpenClaw port (default 18789)"
TEXT_en_agent_port_openfang="Openfang port (default 4200)"
TEXT_en_agent_port_zeroclaw="Zeroclaw port (default 42617)"
TEXT_en_installing_agents="Installing Agent software..."
TEXT_en_agent_install_success="Agent software installed successfully"
TEXT_en_using_registry="Using registry"
TEXT_en_selected_image="Selected image"
TEXT_en_pulling="Pulling image..."
TEXT_en_pull_success="Image pulled successfully"
TEXT_en_pull_failed="Failed to pull image"
TEXT_en_running="Starting container..."
TEXT_en_run_success="Container started successfully"
TEXT_en_run_failed="Failed to start container"
TEXT_en_complete="Deployment Complete"
TEXT_en_access_info="Access Information"
TEXT_en_desktop_url="Desktop"
TEXT_en_agent_port="Agent Port"
TEXT_en_data_dir="Data Directory"
TEXT_en_common_commands="Common Commands"
TEXT_en_view_logs="View logs"
TEXT_en_stop_service="Stop service"
TEXT_en_start_service="Start service"
TEXT_en_enter_container="Enter container"
TEXT_en_check_docker="Checking Docker environment..."
TEXT_en_docker_not_installed="Docker is not installed"
TEXT_en_docker_install_guide="Please install Docker"
TEXT_en_docker_daemon_not_running="Docker daemon is not running"
TEXT_en_start_docker_service="Please start Docker service"
TEXT_en_docker_compose_not_installed="Docker Compose is not installed"
TEXT_en_container_exists="Container already exists"
TEXT_en_delete_recreate="Delete and recreate?"
TEXT_en_deleting_old="Deleting old container..."
TEXT_en_starting_existing="Starting existing container..."
TEXT_en_port_occupied="Port is occupied"
TEXT_en_auto_change_port="Auto changing port to"
TEXT_en_creating_data_dir="Creating data directory..."
TEXT_en_waiting_service="Waiting for service to start..."
TEXT_en_os_detected="Detected OS"
TEXT_en_dind_detected="Detected Docker in Docker (DinD) environment"
TEXT_en_network_mode_title="DinD Network Configuration"
TEXT_en_network_host="1) host mode (recommended, use host network directly)"
TEXT_en_network_bridge="2) bridge mode + hostname mapping"
TEXT_en_select_network="Select network mode"
TEXT_en_using_host_network="Using host network mode"
TEXT_en_using_bridge_network="Using bridge network mode"
TEXT_en_need_sudo="Linux requires root privileges, please run with sudo"
TEXT_en_usage="Usage"
TEXT_en_dind_sudo_warning="Using sudo in DinD environment may cause Docker connection failure"
TEXT_en_try_without_sudo="Please try running without sudo"
TEXT_en_gpu_detecting="Detecting GPU acceleration support..."
TEXT_en_gpu_nvidia="NVIDIA GPU detected"
TEXT_en_gpu_nvidia_runtime="NVIDIA Container Runtime available"
TEXT_en_gpu_nvidia_no_runtime="NVIDIA Container Runtime not found, skipping GPU acceleration"
TEXT_en_gpu_nvidia_toolkit_guide="Please install nvidia-container-toolkit: https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/install-guide.html"
TEXT_en_gpu_intel_amd="Intel/AMD GPU detected (DRI)"
TEXT_en_gpu_none="No GPU detected, using CPU software rendering"
TEXT_en_gpu_enabled="GPU acceleration enabled"
TEXT_en_step_docker_title="Step 3/9: Docker Configuration"
TEXT_en_docker_mode_none="1) Disable Docker (default)"
TEXT_en_docker_mode_dind="2) DinD mode (standalone Docker inside container, requires --privileged)"
TEXT_en_docker_mode_socket="3) Mount host Docker (share host Docker daemon)"
TEXT_en_docker_mode_selected="Docker mode"
TEXT_en_docker_mode_none_desc="Disabled"
TEXT_en_docker_mode_dind_desc="DinD (standalone Docker)"
TEXT_en_docker_mode_socket_desc="Mount host Docker"
TEXT_en_ssh_port="SSH port"
TEXT_en_ssh_password_prompt="Set SSH password (leave empty to disable SSH)"
TEXT_cn_windows_wsl2_detected="检测到 Windows WSL2 环境"
TEXT_cn_windows_wsl2_recommended="推荐使用 WSL2 + Docker 方式运行"
TEXT_cn_windows_wsl2_install_docker="请在 WSL2 中安装 Docker："
TEXT_cn_windows_wsl2_guide="1. 在 WSL2 中执行: sudo apt update && sudo apt install -y docker.io docker-compose"
TEXT_cn_windows_wsl2_guide2="2. 启动 Docker: sudo service docker start"
TEXT_cn_windows_wsl2_guide3="3. 重新运行此脚本"
TEXT_cn_windows_native_detected="检测到 Windows 原生环境"
TEXT_cn_windows_docker_desktop_guide="请安装 Docker Desktop："
TEXT_cn_windows_docker_desktop_url="下载地址: https://www.docker.com/products/docker-desktop"
TEXT_cn_windows_docker_desktop_guide2="安装完成后，请重新运行此脚本"
TEXT_en_windows_wsl2_detected="Windows WSL2 environment detected"
TEXT_en_windows_wsl2_recommended="Recommended to use WSL2 + Docker"
TEXT_en_windows_wsl2_install_docker="Please install Docker in WSL2:"
TEXT_en_windows_wsl2_guide="1. Run in WSL2: sudo apt update && sudo apt install -y docker.io docker-compose"
TEXT_en_windows_wsl2_guide2="2. Start Docker: sudo service docker start"
TEXT_en_windows_wsl2_guide3="3. Re-run this script"
TEXT_en_windows_native_detected="Windows native environment detected"
TEXT_en_windows_docker_desktop_guide="Please install Docker Desktop:"
TEXT_en_windows_docker_desktop_url="Download: https://www.docker.com/products/docker-desktop"
TEXT_en_windows_docker_desktop_guide2="After installation, please re-run this script"
TEXT_cn_macos_detected="检测到 macOS 系统"
TEXT_cn_macos_docker_guide="请安装 Docker，推荐以下方式："
TEXT_cn_macos_orbstack="1) Orbstack (推荐，轻量快速)"
TEXT_cn_macos_orbstack_url="   https://orbstack.dev/"
TEXT_cn_macos_docker_desktop="2) Docker Desktop (官方版)"
TEXT_cn_macos_docker_desktop_url="   https://www.docker.com/products/docker-desktop"
TEXT_cn_macos_podman="3) Podman (开源替代)"
TEXT_cn_macos_podman_url="   https://podman.io/"
TEXT_cn_macos_install_complete="安装完成后，请重新运行此脚本"
TEXT_en_macos_detected="macOS system detected"
TEXT_en_macos_docker_guide="Please install Docker, recommended options:"
TEXT_en_macos_orbstack="1) Orbstack (Recommended, lightweight and fast)"
TEXT_en_macos_orbstack_url="   https://orbstack.dev/"
TEXT_en_macos_docker_desktop="2) Docker Desktop (Official)"
TEXT_en_macos_docker_desktop_url="   https://www.docker.com/products/docker-desktop"
TEXT_en_macos_podman="3) Podman (Open source alternative)"
TEXT_en_macos_podman_url="   https://podman.io/"
TEXT_en_macos_install_complete="After installation, please re-run this script"

# 获取文本函数
get_text() {
    local key="TEXT_${LANG}_${1}"
    echo "${!key}"
}

# ============================================================================
# 配置
# ============================================================================

# 镜像仓库配置
REGISTRY_CN="registry.cn-hangzhou.aliyuncs.com/fliaping/agent-workspace"
REGISTRY_EN="xuping/agent-workspace"


# 默认版本
DEFAULT_VERSION="latest"

# 容器配置
CONTAINER_NAME="agent-workspace"
DESKTOP_PORT="3001"

# 桌面环境（默认 XFCE）
SELECTED_DESKTOP="xfce"

# Agent 软件默认端口配置（容器内端口，不可修改）
AGENT_PORT_openclaw="18789"
AGENT_PORT_openfang="4200"
AGENT_PORT_zeroclaw="42617"

# 用户自定义的 Agent 端口

# 当前语言 (cn/en)
LANG="cn"

# 选择的镜像
SELECTED_IMAGE=""
VERSION=""

# 数据目录
DATA_DIR=""

# 要安装的 Agent 软件
INSTALL_AGENTS=()

# DinD 和网络模式（全局变量，提前设置以便端口检测使用）
IS_DIND=false
USE_HOST_NETWORK=false

# GPU 检测结果
GPU_TYPE=""          # nvidia / intel_amd / none
HAS_NVIDIA_RUNTIME=false
DRINODE_PATH=""

# Docker 模式: none / dind / socket
DOCKER_MODE="none"

# SSH 配置
SSH_PASSWORD=""
SSH_PORT="2222"
DESKTOP_USER="agent"
DESKTOP_PASSWORD=""
# ============================================================================
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

print_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
print_success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
print_warning() { echo -e "${YELLOW}[WARNING]${NC} $1"; }
print_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# ============================================================================
# 步骤 1: 选择语言
# ============================================================================
select_language() {
    echo ""
    echo "========================================"
    echo "  $(get_text welcome_title)"
    echo "========================================"
    echo ""

    echo ""
    print_info "$(get_text step1_title)"
    echo "  $(get_text lang_cn)"
    echo "  $(get_text lang_en)"
    echo ""

    while true; do
        prompt_read lang_choice "$(get_text enter_choice) [1-2, default 1]: "
        lang_choice=${lang_choice:-1}

        case $lang_choice in
            1)
                LANG="cn"
                print_info "Language: 中文"
                return
                ;;
            2)
                LANG="en"
                print_info "Language: English"
                return
                ;;
            *)
                print_error "$(get_text invalid_choice)"
                ;;
        esac
    done
}

# ============================================================================
# 步骤 2: 选择桌面环境
# ============================================================================
select_desktop() {
    echo ""
    print_info "$(get_text step_desktop_title)"
    echo "  $(get_text desktop_xfce)"
    echo "  $(get_text desktop_lxqt)"
    echo "  $(get_text desktop_kde)"
    echo ""

    prompt_read desktop_choice "$(get_text enter_choice) [1-3, default 1]: "
    desktop_choice=${desktop_choice:-1}

    case $desktop_choice in
        1) SELECTED_DESKTOP="xfce" ;;
        2) SELECTED_DESKTOP="lxqt" ;;
        3) SELECTED_DESKTOP="kde" ;;
        *) SELECTED_DESKTOP="xfce" ;;
    esac

    echo ""
    print_info "$(get_text selected_desktop): $SELECTED_DESKTOP"
}

# ============================================================================
# 步骤 3: Docker 配置
# ============================================================================
select_docker_mode() {
    echo ""
    print_info "$(get_text step_docker_title)"
    echo "  $(get_text docker_mode_none)"
    echo "  $(get_text docker_mode_dind)"
    echo "  $(get_text docker_mode_socket)"
    echo ""

    prompt_read docker_choice "$(get_text enter_choice) [1-3, default 1]: "
    docker_choice=${docker_choice:-1}

    case $docker_choice in
        1) DOCKER_MODE="none" ;;
        2) DOCKER_MODE="dind" ;;
        3) DOCKER_MODE="socket" ;;
        *) DOCKER_MODE="none" ;;
    esac

    local mode_desc=""
    case $DOCKER_MODE in
        none)   mode_desc="$(get_text docker_mode_none_desc)" ;;
        dind)   mode_desc="$(get_text docker_mode_dind_desc)" ;;
        socket) mode_desc="$(get_text docker_mode_socket_desc)" ;;
    esac

    echo ""
    print_info "$(get_text docker_mode_selected): $mode_desc"
}

# ============================================================================
# 步骤 4: 选择镜像源
# ============================================================================
select_registry() {
    echo ""
    print_info "$(get_text step_registry_title)"
    echo "  $(get_text registry_cn)"
    echo "  $(get_text registry_en)"
    echo ""

    prompt_read registry_choice "$(get_text enter_choice) [1-2, default 1]: "
    registry_choice=${registry_choice:-1}

    case $registry_choice in
        1)
            REGISTRY="$REGISTRY_CN"
            ;;
        2)
            REGISTRY="$REGISTRY_EN"
            ;;
        *)
            REGISTRY="$REGISTRY_CN"
            ;;
    esac

    echo ""
    print_info "$(get_text using_registry): $REGISTRY"
}

# ============================================================================
# 步骤 4: 选择镜像版本
# ============================================================================
select_version() {
    echo ""
    print_info "$(get_text step2_title)"
    echo "  $(get_text version_latest)"
    echo "  $(get_text version_custom)"
    echo ""

    prompt_read version_choice "$(get_text enter_choice) [1-2, default 1]: "
    version_choice=${version_choice:-1}

    case $version_choice in
        1)
            VERSION="$DEFAULT_VERSION"
            ;;
        2)
            echo ""
            prompt_read VERSION "$(get_text enter_version): "
            ;;
        *)
            VERSION="$DEFAULT_VERSION"
            ;;
    esac

    # 构建完整镜像名：{registry}:ubuntu-{desktop}[-{version}]
    if [ "$VERSION" = "$DEFAULT_VERSION" ]; then
        SELECTED_IMAGE="${REGISTRY}:ubuntu-${SELECTED_DESKTOP}"
    else
        SELECTED_IMAGE="${REGISTRY}:ubuntu-${SELECTED_DESKTOP}-${VERSION}"
    fi

    echo ""
    print_info "$(get_text selected_image): $SELECTED_IMAGE"
}

# ============================================================================
# 步骤 5: 配置数据目录
# ============================================================================
select_data_dir() {
    echo ""
    print_info "$(get_text step3_title)"

    # 获取默认目录（当前用户 home 目录下的 agent-workspace-data）
    local default_dir="$HOME/agent-workspace-data"

    echo ""
    echo "  1) $(get_text data_dir_default): $default_dir"
    echo "  2) $(get_text data_dir_custom)"
    echo ""

    prompt_read dir_choice "$(get_text enter_choice) [1-2, default 1]: "
    dir_choice=${dir_choice:-1}

    case $dir_choice in
        1)
            DATA_DIR="$default_dir"
            ;;
        2)
            echo ""
            prompt_read custom_dir "$(get_text enter_data_dir): "
            if [ -z "$custom_dir" ]; then
                DATA_DIR="$default_dir"
                print_warning "输入为空，使用默认目录: $DATA_DIR"
            else
                DATA_DIR="$custom_dir"
            fi
            ;;
        *)
            DATA_DIR="$default_dir"
            ;;
    esac

    # 展开路径中的 ~
    DATA_DIR="${DATA_DIR/#\~/$HOME}"

    echo ""
    print_info "$(get_text data_dir): $DATA_DIR"
}

# ============================================================================
# 步骤 6: 配置桌面端口
# ============================================================================
select_desktop_port() {
    echo ""
    print_info "$(get_text step4_title)"
    echo ""
    print_info "$(get_text port_desktop): $DESKTOP_PORT"
    echo ""
    echo "  1) $(get_text port_auto)"
    echo "  2) $(get_text port_manual)"
    echo ""

    prompt_read port_choice "$(get_text enter_choice) [1-2, default 1]: "
    port_choice=${port_choice:-1}

    case $port_choice in
        1)
            # 自动检测端口
            print_info "$(get_text port_auto)..."
            if ! check_port_available $DESKTOP_PORT; then
                print_warning "$(get_text port_desktop) $DESKTOP_PORT $(get_text port_in_use)"
                DESKTOP_PORT=$(find_available_port $DESKTOP_PORT)
                print_info "$(get_text port_desktop) $(get_text auto_change_port): $DESKTOP_PORT"
            else
                print_success "$(get_text port_desktop) $DESKTOP_PORT $(get_text port_available)"
            fi
            ;;
        2)
            # 手动配置端口
            echo ""
            prompt_read custom_port "$(get_text enter_desktop_port) [default $DESKTOP_PORT]: "
            if [ -n "$custom_port" ]; then
                if check_port_available $custom_port; then
                    DESKTOP_PORT=$custom_port
                    print_success "$(get_text port_desktop) $DESKTOP_PORT $(get_text port_available)"
                else
                    print_warning "$(get_text port_desktop) $custom_port $(get_text port_in_use)"
                    DESKTOP_PORT=$(find_available_port $custom_port)
                    print_info "$(get_text port_desktop) $(get_text auto_change_port): $DESKTOP_PORT"
                fi
            fi
            ;;
        *)
            # 默认自动检测
            if ! check_port_available $DESKTOP_PORT; then
                DESKTOP_PORT=$(find_available_port $DESKTOP_PORT)
            fi
            ;;
    esac

    # HTTPS 端口
    echo ""
    print_info "$(get_text port_desktop) (HTTPS): $DESKTOP_PORT"
}

# ============================================================================
# SSH 配置
# ============================================================================
configure_ssh() {
    echo ""
    print_info "$(get_text ssh_password_prompt)"
    echo ""
    prompt_read ssh_pass "$(get_text ssh_password_prompt): "

    if [ -z "$ssh_pass" ]; then
        SSH_PASSWORD=""
        return
    fi

    SSH_PASSWORD="$ssh_pass"

    echo ""
    prompt_read custom_ssh_port "$(get_text ssh_port) [default $SSH_PORT]: "
    if [ -n "$custom_ssh_port" ]; then
        SSH_PORT="$custom_ssh_port"
    fi

    echo ""
    print_info "$(get_text ssh_port): $SSH_PORT"
}

# ============================================================================
# 步骤 8: 配置 Agent 软件端口
# ============================================================================
configure_agent_ports() {
    if [ ${#INSTALL_AGENTS[@]} -eq 0 ]; then
        return
    fi

    local has_service=false
    for agent in "${INSTALL_AGENTS[@]}"; do
        case "$agent" in openclaw|openfang|zeroclaw) has_service=true ;; esac
    done
    if [ "$has_service" = false ]; then
        return
    fi

    echo ""
    print_info "$(get_text step6_title)"
    echo ""
    print_info "$(get_text agent_port_config)"
    echo ""

    for agent in "${INSTALL_AGENTS[@]}"; do
        case "$agent" in openclaw|openfang|zeroclaw) ;; *) continue ;; esac
        local default_port="$(eval echo \$AGENT_PORT_${agent})"
        local port_name=""

        case $agent in
            openclaw) port_name="$(get_text agent_port_openclaw)" ;;
            openfang) port_name="$(get_text agent_port_openfang)" ;;
            zeroclaw) port_name="$(get_text agent_port_zeroclaw)" ;;
        esac

        echo ""
        echo "  $port_name"
        echo "  1) $(get_text agent_port_default): $default_port"
        echo "  2) $(get_text agent_port_custom)"
        echo ""

        prompt_read port_choice "$(get_text enter_choice) [1-2, default 1]: "
        port_choice=${port_choice:-1}

        case $port_choice in
            1)
                eval "CUSTOM_PORT_${agent}=\$default_port"
                # 检查端口是否可用
                if ! check_port_available $default_port; then
                    print_warning "$port_name $default_port $(get_text port_in_use)"
                    local new_port=$(find_available_port $default_port)
                    eval "CUSTOM_PORT_${agent}=\$new_port"
                    print_info "$port_name $(get_text auto_change_port): $new_port"
                else
                    print_success "$port_name $default_port $(get_text port_available)"
                fi
                ;;
            2)
                echo ""
                prompt_read custom_port "$(get_text enter_agent_port_num): "
                if [ -n "$custom_port" ]; then
                    if check_port_available $custom_port; then
                        eval "CUSTOM_PORT_${agent}=\$custom_port"
                        print_success "$port_name $custom_port $(get_text port_available)"
                    else
                        print_warning "$port_name $custom_port $(get_text port_in_use)"
                        local new_port=$(find_available_port $custom_port)
                        eval "CUSTOM_PORT_${agent}=\$new_port"
                        print_info "$port_name $(get_text auto_change_port): $new_port"
                    fi
                else
                    eval "CUSTOM_PORT_${agent}=\$default_port"
                fi
                ;;
            *)
                eval "CUSTOM_PORT_${agent}=\$default_port"
                ;;
        esac
    done

    # 显示配置的端口
    echo ""
    print_info "Agent 软件端口配置:"
    for agent in "${INSTALL_AGENTS[@]}"; do
        case "$agent" in openclaw|openfang|zeroclaw) ;; *) continue ;; esac
        echo "  $agent: $(eval echo \$CUSTOM_PORT_${agent})"
    done
}

# ============================================================================
# 步骤 7: 选择要安装的 Agent 软件
# ============================================================================
select_agents() {
    echo ""
    print_info "$(get_text step5_title)"
    echo ""
    print_info "$(get_text agent_install_title)"
    echo "  $(get_text agent_codex)"
    echo "  $(get_text agent_claude_code)"
    echo "  $(get_text agent_hermes)"
    echo "  $(get_text agent_openclaw)"
    echo "  $(get_text agent_openfang)"
    echo "  $(get_text agent_zeroclaw)"
    echo "  $(get_text agent_skip)"
    echo ""

    prompt_read agent_choices "$(get_text enter_agents) [1-7, default 1]: "
    agent_choices=${agent_choices:-1}

    # 解析用户选择
    for choice in $agent_choices; do
        case $choice in
            1) INSTALL_AGENTS+=("codex") ;;
            2) INSTALL_AGENTS+=("claude-code") ;;
            3) INSTALL_AGENTS+=("hermes") ;;
            4) INSTALL_AGENTS+=("openclaw") ;;
            5) INSTALL_AGENTS+=("openfang") ;;
            6) INSTALL_AGENTS+=("zeroclaw") ;;
            7)
                INSTALL_AGENTS=()
                print_info "跳过 Agent 软件安装"
                return
                ;;
        esac
    done

    if [ ${#INSTALL_AGENTS[@]} -gt 0 ]; then
        echo ""
        print_info "将安装以下软件: ${INSTALL_AGENTS[*]}"
    fi
}

# ============================================================================
# 在容器内安装 Agent 软件
# ============================================================================
install_agents_in_container() {
    local flags="--non-interactive"
    if [ "$LANG" = "cn" ]; then
        flags="$flags --china-mirror"
    fi

    # Install as the desktop user (abc); running as root leaves root-owned
    # files such as /config/.codex that the Agent cannot write at login.
    docker exec -u abc -e HOME=/config "$CONTAINER_NAME" \
        install-agent.sh $flags "${INSTALL_AGENTS[@]}" \
        || print_warning "Agent installation failed, please install manually"

    print_success "$(get_text agent_install_success)"
}

# ============================================================================
# 环境检测
# ============================================================================

# 检测操作系统
detect_os() {
    if [[ "$OSTYPE" == "linux-gnu"* ]]; then
        if [ -f /etc/os-release ]; then
            . /etc/os-release
            OS=$NAME
            OS_TYPE="linux"
        else
            OS="Linux"
            OS_TYPE="linux"
        fi
    elif [[ "$OSTYPE" == "darwin"* ]]; then
        OS="macOS"
        OS_TYPE="macos"
    elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]] || [[ "$OSTYPE" == "win32" ]]; then
        OS="Windows"
        OS_TYPE="windows"
    else
        OS="Unknown"
        OS_TYPE="unknown"
    fi
}

# 检测 GPU 加速支持
detect_gpu() {
    print_info "$(get_text gpu_detecting)"

    # DinD 环境下跳过 GPU 检测（无法直接访问宿主机设备）
    if [ "$IS_DIND" = true ]; then
        GPU_TYPE="none"
        print_info "$(get_text gpu_none) (DinD)"
        return
    fi

    # 1. 检测 NVIDIA GPU
    if command -v nvidia-smi &> /dev/null && nvidia-smi &> /dev/null; then
        GPU_TYPE="nvidia"
        local gpu_name
        gpu_name=$(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1)
        print_success "$(get_text gpu_nvidia): ${gpu_name}"

        # 检测 NVIDIA Container Runtime
        if docker info 2>/dev/null | grep -qi "nvidia"; then
            HAS_NVIDIA_RUNTIME=true
            print_success "$(get_text gpu_nvidia_runtime)"
        else
            HAS_NVIDIA_RUNTIME=false
            print_warning "$(get_text gpu_nvidia_no_runtime)"
            print_info "$(get_text gpu_nvidia_toolkit_guide)"
        fi

        # NVIDIA 也有 /dev/dri
        if [ -d /dev/dri ]; then
            DRINODE_PATH=$(ls /dev/dri/renderD* 2>/dev/null | head -1)
        fi
        return
    fi

    # 2. 检测 Intel/AMD GPU (DRI 设备)
    if [ -d /dev/dri ] && ls /dev/dri/renderD* &> /dev/null; then
        GPU_TYPE="intel_amd"
        DRINODE_PATH=$(ls /dev/dri/renderD* 2>/dev/null | head -1)
        local gpu_info=""
        if command -v lspci &> /dev/null; then
            gpu_info=$(lspci 2>/dev/null | grep -iE "VGA|3D|Display" | head -1 | sed 's/.*: //')
        fi
        print_success "$(get_text gpu_intel_amd): ${gpu_info:-${DRINODE_PATH}}"
        return
    fi

    # 3. macOS（Docker Desktop 不支持 GPU 直通）
    # 4. 无 GPU
    GPU_TYPE="none"
    print_info "$(get_text gpu_none)"
}

# 检测是否在 WSL2 环境
check_wsl2() {
    if [ -f /proc/version ]; then
        if grep -q "Microsoft" /proc/version || grep -q "WSL" /proc/version; then
            return 0
        fi
    fi
    return 1
}

# 检测 Windows/macOS 环境并给出指引
check_windows_environment() {
    if [[ "$OS_TYPE" == "windows" ]]; then
        # 检查是否在 WSL2 中
        if check_wsl2; then
            print_success "$(get_text windows_wsl2_detected)"
            print_info "$(get_text windows_wsl2_recommended)"

            # 检查 Docker 是否已安装
            if ! command -v docker &> /dev/null; then
                echo ""
                print_error "$(get_text docker_not_installed)"
                print_info "$(get_text windows_wsl2_install_docker)"
                echo ""
                echo "  $(get_text windows_wsl2_guide)"
                echo "  $(get_text windows_wsl2_guide2)"
                echo "  $(get_text windows_wsl2_guide3)"
                echo ""
                exit 1
            fi
        else
            # Windows 原生环境
            print_warning "$(get_text windows_native_detected)"

            # 检查 Docker Desktop 是否已安装
            if ! command -v docker &> /dev/null; then
                echo ""
                print_error "$(get_text docker_not_installed)"
                print_info "$(get_text windows_docker_desktop_guide)"
                echo ""
                echo "  $(get_text windows_docker_desktop_url)"
                echo ""
                echo "  $(get_text windows_docker_desktop_guide2)"
                echo ""
                exit 1
            fi
        fi
    elif [[ "$OS_TYPE" == "macos" ]]; then
        # macOS 环境
        print_info "$(get_text macos_detected)"

        # 检查 Docker 是否已安装
        if ! command -v docker &> /dev/null; then
            echo ""
            print_error "$(get_text docker_not_installed)"
            print_info "$(get_text macos_docker_guide)"
            echo ""
            echo "  $(get_text macos_orbstack)"
            echo "     $(get_text macos_orbstack_url)"
            echo ""
            echo "  $(get_text macos_docker_desktop)"
            echo "     $(get_text macos_docker_desktop_url)"
            echo ""
            echo "  $(get_text macos_podman)"
            echo "     $(get_text macos_podman_url)"
            echo ""
            echo "  $(get_text macos_install_complete)"
            echo ""
            exit 1
        fi
    fi
}

# 检测是否在容器内
is_container() {
    [ -f /.dockerenv ] || grep -qE "docker|kubepods" /proc/1/cgroup 2>/dev/null
}

# 检测是否在 DinD 环境
check_dind() {
    local score=0
    [ -f /.dockerenv ] && score=$((score + 1))
    if grep -qE "docker|kubepods" /proc/1/cgroup 2>/dev/null || [ "$(cat /proc/1/cgroup 2>/dev/null | head -1)" = "0::/" ]; then
        score=$((score + 1))
    fi
    [ -S /var/run/docker.sock ] && score=$((score + 1))
    local init_proc=$(ps -p 1 -o comm= 2>/dev/null)
    [ "$init_proc" != "systemd" ] && [ "$init_proc" != "init" ] && score=$((score + 1))
    local hostname_val=$(hostname 2>/dev/null)
    echo "$hostname_val" | grep -qE "^[a-f0-9]{12}$" && score=$((score + 1))
    [ $score -ge 3 ]
}

# ============================================================================
# Docker 相关
# ============================================================================

check_docker() {
    print_info "$(get_text check_docker)"
    if command -v docker &> /dev/null; then
        DOCKER_VERSION=$(docker --version | awk '{print $3}' | sed 's/,//')
        print_success "Docker: $DOCKER_VERSION"
        return 0
    else
        return 1
    fi
}

check_docker_compose() {
    if docker compose version &> /dev/null; then
        return 0
    elif docker-compose --version &> /dev/null; then
        return 0
    else
        return 1
    fi
}

check_docker_daemon() {
    if docker ps &> /dev/null; then
        return 0
    fi
    if docker info &> /dev/null; then
        return 0
    fi
    return 1
}

# ============================================================================
# 端口管理
# ============================================================================

check_port_available() {
    local port=$1
    # 在 DinD 环境中，端口检测看到的是容器自己的端口空间，不是宿主机的
    if [ "$IS_DIND" = true ]; then
        return 0
    fi
    if command -v ss &> /dev/null; then
        ! ss -tuln 2>/dev/null | grep -q ":$port "
    elif command -v netstat &> /dev/null; then
        ! netstat -tuln 2>/dev/null | grep -q ":$port "
    else
        return 0
    fi
}

find_available_port() {
    local base_port=$1
    local port=$base_port
    while ! check_port_available $port; do
        port=$((port + 1))
        if [ $port -gt $((base_port + 100)) ]; then
            print_error "$(get_text port_occupied)"
            exit 1
        fi
    done
    echo $port
}

# ============================================================================
# 主流程
# ============================================================================

main() {
    # 先检测环境（DinD 检测需要在端口选择之前完成）
    detect_os
    print_info "$(get_text os_detected): $OS"

    # Windows 环境特殊处理
    check_windows_environment

    # 检查是否在 DinD 环境（设置全局变量）
    if check_dind; then
        IS_DIND=true
        print_warning "$(get_text dind_detected)"
    fi

    # 检查 Docker
    if ! check_docker; then
        print_error "$(get_text docker_not_installed)"
        if [[ "$OS_TYPE" == "linux" ]]; then
            print_info "$(get_text docker_install_guide): https://docs.docker.com/engine/install/"
        elif [[ "$OS_TYPE" == "windows" ]]; then
            exit 1
        else
            print_info "$(get_text docker_install_guide): https://www.docker.com/products/docker-desktop"
        fi
        exit 1
    fi

    # 检查 Docker Compose
    if ! check_docker_compose; then
        print_warning "$(get_text docker_compose_not_installed)"
    fi

    # 检查 Docker 守护进程
    if ! check_docker_daemon; then
        print_error "$(get_text docker_not_running)"
        print_info "$(get_text start_docker_service): sudo systemctl start docker"
        exit 1
    fi

    # 检测 GPU 加速
    detect_gpu

    # 步骤 1: 选择语言
    select_language

    # 步骤 2: 选择桌面环境
    select_desktop

    # 步骤 3: Docker 配置
    select_docker_mode

    # 步骤 4: 选择镜像源
    select_registry

    # 步骤 5: 选择版本
    select_version

    # 步骤 6: 配置数据目录
    select_data_dir

    # DinD 环境网络模式选择（在端口配置之前）
    if [ "$IS_DIND" = true ]; then
        echo ""
        print_info "$(get_text network_mode_title)"
        echo "  $(get_text network_host)"
        echo "  $(get_text network_bridge)"
        echo ""
        prompt_read network_mode "$(get_text select_network) [1/2, default 2]: "
        network_mode=${network_mode:-2}
        if [[ "$network_mode" == "1" ]]; then
            USE_HOST_NETWORK=true
            print_info "$(get_text using_host_network)"
        else
            USE_HOST_NETWORK=false
            print_info "$(get_text using_bridge_network)"
        fi
    fi

    # 步骤 6: 配置桌面端口（host 模式下跳过）
    if [ "$USE_HOST_NETWORK" = false ]; then
        select_desktop_port
    else
        print_info "host 网络模式，使用容器内默认端口 3001 (HTTPS)"
    fi

    # SSH 配置
    configure_ssh

    # 步骤 7: 选择 Agent 软件
    select_agents

    # 步骤 8: 配置 Agent 端口
    configure_agent_ports

    # 检查/删除旧容器
    if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
        print_warning "$(get_text container_exists): $CONTAINER_NAME"
        prompt_read recreate "$(get_text delete_recreate) [y/N]: "
        if [[ "$recreate" =~ ^[Yy]$ ]]; then
            print_info "$(get_text deleting_old)"
            docker stop "$CONTAINER_NAME" &> /dev/null || true
            docker rm "$CONTAINER_NAME" &> /dev/null || true
        else
            print_info "$(get_text starting_existing)"
            docker start "$CONTAINER_NAME"
            print_success "$(get_text run_success)"
            print_access_info
            return
        fi
    fi

    # 端口检查（非 host 模式）
    if [ "$USE_HOST_NETWORK" = false ]; then
        if ! check_port_available $DESKTOP_PORT; then
            print_warning "$(get_text port_occupied): $DESKTOP_PORT"
            DESKTOP_PORT=$(find_available_port $DESKTOP_PORT)
            print_info "$(get_text auto_change_port): $DESKTOP_PORT"
        fi
        # SSH 端口
        if [ -n "$SSH_PASSWORD" ] && ! check_port_available $SSH_PORT; then
            print_warning "$(get_text port_occupied): $SSH_PORT"
            SSH_PORT=$(find_available_port $SSH_PORT)
            print_info "$(get_text auto_change_port): $SSH_PORT"
        fi
    fi

    # 创建数据目录
    print_info "$(get_text creating_data_dir)"
    mkdir -p "$DATA_DIR"

    # 拉取镜像
    print_info "$(get_text pulling)"
    if ! docker pull "$SELECTED_IMAGE"; then
        print_error "$(get_text pull_failed)"
        exit 1
    fi
    print_success "$(get_text pull_success)"

    # 构建 docker run 命令
    DESKTOP_PASSWORD="$(head -c 24 /dev/urandom | base64 | tr -d '\n')"
    DOCKER_ARGS=(
        "-d"
        "--name" "$CONTAINER_NAME"
        "--restart" "unless-stopped"
        "--shm-size" "2gb"
        "-e" "PUID=1000"
        "-e" "PGID=1000"
        "-e" "TZ=Asia/Taipei"
        "-e" "LC_ALL=zh_CN.UTF-8"
        "-e" "CUSTOM_USER=${DESKTOP_USER}"
        "-e" "PASSWORD=${DESKTOP_PASSWORD}"
        "-e" "NODE_OPTIONS=--max-old-space-size=2048"
        "-e" "SELKIES_ENABLE_RATE_CONTROL=true"
        "-e" "SELKIES_RATE_CONTROL_MODE=crf,cbr"
        "-e" "SELKIES_CONGESTION_CONTROL=false"
        "-e" "SELKIES_ENABLE_RESIZE=true"
        "-v" "${DATA_DIR}:/config"
    )

    # Docker 模式配置
    case $DOCKER_MODE in
        dind)
            DOCKER_ARGS+=(
                "--privileged"
                "-e" "START_DOCKER=true"
            )
            ;;
        socket)
            DOCKER_ARGS+=(
                "-v" "/var/run/docker.sock:/var/run/docker.sock"
                "-e" "START_DOCKER=false"
            )
            ;;
        none)
            DOCKER_ARGS+=("-e" "START_DOCKER=false")
            ;;
    esac

    # GPU 加速配置
    if [ "$GPU_TYPE" = "nvidia" ] && [ "$HAS_NVIDIA_RUNTIME" = true ]; then
        # NVIDIA GPU: --gpus all + 环境变量
        DOCKER_ARGS+=(
            "--gpus" "all"
            "-e" "NVIDIA_VISIBLE_DEVICES=all"
            "-e" "NVIDIA_DRIVER_CAPABILITIES=all"
        )
        if [ -d /dev/dri ]; then
            DOCKER_ARGS+=("--device" "/dev/dri:/dev/dri")
        fi
        if [ -n "$DRINODE_PATH" ]; then
            DOCKER_ARGS+=(
                "-e" "DRINODE=${DRINODE_PATH}"
                "-e" "DRI_NODE=${DRINODE_PATH}"
            )
        fi
        print_success "$(get_text gpu_enabled) (NVIDIA)"
    elif [ "$GPU_TYPE" = "intel_amd" ]; then
        # Intel/AMD GPU: DRI 设备直通
        DOCKER_ARGS+=(
            "--device" "/dev/dri:/dev/dri"
            "-e" "DRINODE=${DRINODE_PATH:-/dev/dri/renderD128}"
            "-e" "DRI_NODE=${DRINODE_PATH:-/dev/dri/renderD128}"
        )
        print_success "$(get_text gpu_enabled) (Intel/AMD)"
    fi

    # SSH（密码设置时启用）
    if [ -n "$SSH_PASSWORD" ]; then
        DOCKER_ARGS+=("-e" "SSH_PASSWORD=${SSH_PASSWORD}")
        if [ "$USE_HOST_NETWORK" = false ]; then
            DOCKER_ARGS+=("-p" "${SSH_PORT}:22")
        fi
    fi

    # 网络配置
    if [ "$USE_HOST_NETWORK" = true ]; then
        DOCKER_ARGS+=("--network" "host")
    else
        DOCKER_ARGS+=(
            "-p" "${DESKTOP_PORT}:3001"
        )
        if [ "$IS_DIND" = true ]; then
            DOCKER_ARGS+=(
                "--add-host" "agent-workspace:127.0.0.1"
                "--hostname" "agent-workspace"
            )
        fi
    fi

    # 添加 Agent 软件端口映射
    if [ ${#INSTALL_AGENTS[@]} -gt 0 ] && [ "$USE_HOST_NETWORK" = false ]; then
        for agent in "${INSTALL_AGENTS[@]}"; do
            case "$agent" in openclaw|openfang|zeroclaw) ;; *) continue ;; esac
            local custom_port="$(eval echo \$CUSTOM_PORT_${agent})"
            local internal_port="$(eval echo \$AGENT_PORT_${agent})"
            DOCKER_ARGS+=("-p" "${custom_port}:${internal_port}")
        done
    fi

    DOCKER_ARGS+=("$SELECTED_IMAGE")

    # 启动容器
    print_info "$(get_text running)"
    if ! docker run "${DOCKER_ARGS[@]}"; then
        print_error "$(get_text run_failed)"
        exit 1
    fi

    # 等待服务就绪（HTTP 探测）
    print_info "$(get_text waiting_service)"
    local max_wait=120
    local waited=0 http_code=""
    while [ $waited -lt $max_wait ]; do
        # Port 3000 answers 401 when CUSTOM_USER/PASSWORD are set (always here),
        # so treat any HTTP response as ready, like the image HEALTHCHECK.
        http_code="$(docker exec "$CONTAINER_NAME" curl -s -o /dev/null -w '%{http_code}' http://localhost:3000/ 2>/dev/null || true)"
        if [[ "$http_code" =~ ^[234][0-9][0-9]$ ]]; then
            break
        fi
        sleep 3
        waited=$((waited + 3))
        echo -n "."
    done
    echo ""

    # 如果有 Agent 软件需要安装，在容器内直接执行安装命令
    if [ ${#INSTALL_AGENTS[@]} -gt 0 ]; then
        echo ""
        print_info "$(get_text installing_agents)"
        install_agents_in_container
    fi

    print_success "$(get_text complete)!"
    print_access_info
}

# ============================================================================
# 输出访问信息
# ============================================================================

print_access_info() {
    echo ""
    echo "========================================"
    echo "  $(get_text access_info)"
    echo "========================================"
    echo ""

    local IP=""
    if command -v hostname &> /dev/null; then
        # `hostname -I` is Linux-only (fails on macOS); the pipeline's status is
        # awk's, so fall back explicitly when nothing was printed.
        IP=$(hostname -I 2>/dev/null | awk '{print $1}')
    fi
    IP="${IP:-localhost}"

    if [ "$USE_HOST_NETWORK" = true ]; then
        print_info "🖥️  $(get_text desktop_url) (HTTPS): https://${IP}:3001/"
    else
        print_info "🖥️  $(get_text desktop_url) (HTTPS): https://${IP}:${DESKTOP_PORT}/"
    fi

    if [ -n "$DESKTOP_PASSWORD" ]; then
        print_info "👤 Webtop: ${DESKTOP_USER}"
        print_info "🔐 Password: ${DESKTOP_PASSWORD}"
    fi

    print_info "💾 $(get_text data_dir): ${DATA_DIR}"

    # 显示 SSH 连接信息
    if [ -n "$SSH_PASSWORD" ]; then
        if [ "$USE_HOST_NETWORK" = true ]; then
            print_info "🔑 SSH: ssh abc@${IP} -p 22"
        else
            print_info "🔑 SSH: ssh abc@${IP} -p ${SSH_PORT}"
        fi
    fi

    # 显示 Docker 模式
    case $DOCKER_MODE in
        dind)   print_info "🐳 Docker: DinD ($(get_text docker_mode_dind_desc))" ;;
        socket) print_info "🐳 Docker: $(get_text docker_mode_socket_desc)" ;;
        none)   print_info "🐳 Docker: $(get_text docker_mode_none_desc)" ;;
    esac

    if [ ${#INSTALL_AGENTS[@]} -gt 0 ]; then
        echo ""
        print_info "📦 已安装 Agent 软件:"
        for agent in "${INSTALL_AGENTS[@]}"; do
            case "$agent" in
                codex) echo "    ✓ codex — 运行 codex 登录" ;;
                claude-code) echo "    ✓ claude-code — 运行 claude 登录" ;;
                hermes) echo "    ✓ hermes — 运行 hermes setup --portal 配置" ;;
                *)
                    local custom_port="$(eval echo \$CUSTOM_PORT_${agent})"
                    local internal_port="$(eval echo \$AGENT_PORT_${agent})"
                    if [ "$USE_HOST_NETWORK" = true ]; then
                        echo "    ✓ $agent (端口: $internal_port)"
                    else
                        echo "    ✓ $agent (宿主机:$custom_port → 容器:$internal_port)"
                    fi
                    ;;
            esac
        done
        echo ""
        print_info "🔧 容器管理入口: docker exec $CONTAINER_NAME workspacectl info"
    fi

    echo ""
    print_info "📋 $(get_text common_commands):"
    echo "    $(get_text view_logs): docker logs -f $CONTAINER_NAME"
    echo "    $(get_text stop_service): docker stop $CONTAINER_NAME"
    echo "    $(get_text start_service): docker start $CONTAINER_NAME"
    echo "    $(get_text enter_container): docker exec -it $CONTAINER_NAME bash"
    echo "========================================"
}

# ============================================================================
# 入口
# ============================================================================

# 检查是否以 sudo 运行（Linux，且非 DinD 环境）
if [[ "$OSTYPE" == "linux-gnu"* ]] && [ "$EUID" -ne 0 ] && ! check_dind; then
    print_error "$(get_text need_sudo)"
    echo ""
    echo "$(get_text usage):"
    echo "  sudo ./install.sh"
    echo "  $(get_text or)"
    echo '  curl -fsSL https://raw.githubusercontent.com/fliaping/cranibox/main/install.sh | sudo bash'
    exit 1
fi

# DinD 环境特殊处理
if check_dind && [ "$EUID" -eq 0 ]; then
    print_warning "$(get_text dind_sudo_warning)"
    print_info "$(get_text try_without_sudo): ./install.sh"
    exit 1
fi

main "$@"
