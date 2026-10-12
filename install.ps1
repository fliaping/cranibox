#
# Agent Workspace One-Click Deploy (Windows PowerShell)
# Based on LinuxServer webtop (Selkies)
# Requires: Windows 10/11 + Docker Desktop
# Project: https://github.com/fliaping/cranibox
#
# Usage:
#   .\install.ps1
#   # or remote:
#   irm https://raw.githubusercontent.com/fliaping/cranibox/main/install.ps1 | iex
#

$ErrorActionPreference = "Stop"

# ============================================================================
# i18n texts
# ============================================================================
$Texts = @{
    cn = @{
        welcome_title = "Agent Workspace 一键部署"
        step1_title = "步骤 1/9: 选择语言"
        lang_cn = "1) 中文 (Chinese)"
        lang_en = "2) English"
        step_desktop_title = "步骤 2/9: 选择桌面环境"
        desktop_lxqt = "2) LXQt  — 轻量 (~300MB 内存)"
        desktop_xfce = "1) XFCE  — 推荐 (~800MB 内存)"
        desktop_kde  = "3) KDE   — 完整 (~1.1GB 内存)"
        selected_desktop = "已选择桌面"
        step_docker_title = "步骤 3/9: Docker 配置"
        docker_mode_none = "1) 不启用 Docker（默认）"
        docker_mode_dind = "2) DinD 模式（容器内独立 Docker，需要 privileged）"
        docker_mode_socket = "3) 挂载宿主机 Docker（共享 Docker Desktop）"
        docker_mode_selected = "Docker 模式"
        docker_mode_none_desc = "不启用"
        docker_mode_dind_desc = "DinD（独立 Docker）"
        docker_mode_socket_desc = "挂载宿主机 Docker"
        ssh_port = "SSH 端口"
        ssh_password_prompt = "设置 SSH 密码（留空则不启用 SSH）"
        step_registry_title = "步骤 4/9: 选择镜像源"
        registry_cn = "1) 阿里云镜像（国内推荐）"
        registry_en = "2) Docker Hub（海外推荐）"
        step_version_title = "步骤 5/9: 选择镜像版本"
        version_latest = "1) latest (最新版)"
        version_custom = "2) 自定义版本"
        enter_version = "请输入版本号"
        step_data_dir_title = "步骤 6/9: 配置数据目录"
        data_dir_default = "使用默认目录"
        data_dir_custom = "自定义目录"
        enter_data_dir = "请输入数据目录路径"
        step_port_title = "步骤 7/9: 配置桌面端口"
        port_desktop = "桌面端口 (HTTPS)"
        port_auto = "自动检测并分配端口"
        port_manual = "手动配置端口"
        enter_desktop_port = "请输入桌面端口"
        port_in_use = "端口已被占用"
        port_available = "端口可用"
        auto_change_port = "自动更换端口为"
        step_agent_title = "步骤 8/9: 选择 Agent 软件"
        agent_install_title = "选择要安装的 Agent 软件（可多选，空格分隔）"
        agent_codex = "1) Codex - OpenAI 编程 Agent（推荐，安装后运行 codex 登录）"
        agent_claude_code = "2) Claude Code - Anthropic 编程 Agent（安装后运行 claude 登录）"
        agent_hermes = "3) Hermes Agent - 通用自主 Agent（运行 hermes setup --portal）"
        agent_openclaw = "4) OpenClaw - 个人自主开源 AI 助手"
        agent_openfang = "5) Openfang - Rust Agent OS"
        agent_zeroclaw = "6) Zeroclaw - 超轻量 Agent 运行时"
        agent_skip = "7) 跳过，不安装"
        enter_agents = "请输入选项（如：1 2 3）"
        step_agent_port_title = "步骤 9/9: 配置 Agent 软件端口"
        agent_port_default = "使用默认端口"
        agent_port_custom = "自定义端口"
        enter_agent_port = "请输入端口"
        agent_port_openclaw = "OpenClaw 端口（默认 18789）"
        agent_port_openfang = "Openfang 端口（默认 4200）"
        agent_port_zeroclaw = "Zeroclaw 端口（默认 42617）"
        enter_choice = "请输入选项"
        invalid_choice = "无效选项，请重新输入"
        using_registry = "使用镜像仓库"
        selected_image = "已选择镜像"
        pulling = "正在拉取镜像..."
        pull_success = "镜像拉取成功"
        pull_failed = "镜像拉取失败"
        running = "正在启动容器..."
        run_success = "容器启动成功"
        run_failed = "容器启动失败"
        complete = "部署完成"
        access_info = "访问信息"
        desktop_url = "桌面"
        data_dir = "数据目录"
        common_commands = "常用命令"
        view_logs = "查看日志"
        stop_service = "停止服务"
        start_service = "启动服务"
        enter_container = "进入容器"
        check_docker = "检查 Docker 环境..."
        docker_not_installed = "Docker 未安装"
        docker_install_guide = "请安装 Docker Desktop"
        docker_daemon_not_running = "Docker Desktop 未运行"
        start_docker_service = "请启动 Docker Desktop"
        docker_compose_not_installed = "Docker Compose 未安装"
        container_exists = "容器已存在"
        delete_recreate = "是否删除并重新创建? [y/N]"
        deleting_old = "正在删除旧容器..."
        starting_existing = "正在启动已有容器..."
        creating_data_dir = "正在创建数据目录..."
        waiting_service = "等待服务启动..."
        installing_agents = "正在安装 Agent 软件..."
        agent_install_success = "Agent 软件安装成功"
        gpu_not_supported = "Windows Docker Desktop 不支持 GPU 直通"
    }
    en = @{
        welcome_title = "Agent Workspace Deployment"
        step1_title = "Step 1/9: Select Language"
        lang_cn = "1) Chinese"
        lang_en = "2) English"
        step_desktop_title = "Step 2/9: Select Desktop"
        desktop_lxqt = "2) LXQt  — Lightweight (~300MB RAM)"
        desktop_xfce = "1) XFCE  — Recommended (~800MB RAM)"
        desktop_kde  = "3) KDE   — Full (~1.1GB RAM)"
        selected_desktop = "Selected desktop"
        step_docker_title = "Step 3/9: Docker Configuration"
        docker_mode_none = "1) Disable Docker (default)"
        docker_mode_dind = "2) DinD mode (standalone Docker, requires privileged)"
        docker_mode_socket = "3) Mount host Docker (share Docker Desktop)"
        docker_mode_selected = "Docker mode"
        docker_mode_none_desc = "Disabled"
        docker_mode_dind_desc = "DinD (standalone Docker)"
        docker_mode_socket_desc = "Mount host Docker"
        ssh_port = "SSH port"
        ssh_password_prompt = "Set SSH password (leave empty to disable SSH)"
        step_registry_title = "Step 4/9: Select Registry"
        registry_cn = "1) Alibaba Cloud (China)"
        registry_en = "2) Docker Hub (International)"
        step_version_title = "Step 5/9: Select Version"
        version_latest = "1) latest"
        version_custom = "2) Custom version"
        enter_version = "Enter version tag"
        step_data_dir_title = "Step 6/9: Configure Data Directory"
        data_dir_default = "Use default directory"
        data_dir_custom = "Custom directory"
        enter_data_dir = "Enter data directory path"
        step_port_title = "Step 7/9: Configure Desktop Port"
        port_desktop = "Desktop Port (HTTPS)"
        port_auto = "Auto detect and assign port"
        port_manual = "Manually configure port"
        enter_desktop_port = "Enter desktop port"
        port_in_use = "Port is in use"
        port_available = "Port is available"
        auto_change_port = "Auto changing port to"
        step_agent_title = "Step 8/9: Select Agent Software"
        agent_install_title = "Select Agent software (space separated)"
        agent_codex = "1) Codex - OpenAI coding agent (recommended; run codex to sign in)"
        agent_claude_code = "2) Claude Code - Anthropic coding agent (run claude to sign in)"
        agent_hermes = "3) Hermes Agent - General autonomous agent (run hermes setup --portal)"
        agent_openclaw = "4) OpenClaw - AI assistant"
        agent_openfang = "5) Openfang - Rust Agent OS"
        agent_zeroclaw = "6) Zeroclaw - Ultra-light Agent"
        agent_skip = "7) Skip"
        enter_agents = "Enter options (e.g., 1 2 3)"
        step_agent_port_title = "Step 9/9: Configure Agent Ports"
        agent_port_default = "Use default port"
        agent_port_custom = "Custom port"
        enter_agent_port = "Enter port"
        agent_port_openclaw = "OpenClaw port (default 18789)"
        agent_port_openfang = "Openfang port (default 4200)"
        agent_port_zeroclaw = "Zeroclaw port (default 42617)"
        enter_choice = "Enter your choice"
        invalid_choice = "Invalid choice, please try again"
        using_registry = "Using registry"
        selected_image = "Selected image"
        pulling = "Pulling image..."
        pull_success = "Image pulled successfully"
        pull_failed = "Failed to pull image"
        running = "Starting container..."
        run_success = "Container started successfully"
        run_failed = "Failed to start container"
        complete = "Deployment Complete"
        access_info = "Access Information"
        desktop_url = "Desktop"
        data_dir = "Data Directory"
        common_commands = "Common Commands"
        view_logs = "View logs"
        stop_service = "Stop service"
        start_service = "Start service"
        enter_container = "Enter container"
        check_docker = "Checking Docker environment..."
        docker_not_installed = "Docker is not installed"
        docker_install_guide = "Please install Docker Desktop"
        docker_daemon_not_running = "Docker Desktop is not running"
        start_docker_service = "Please start Docker Desktop"
        docker_compose_not_installed = "Docker Compose is not installed"
        container_exists = "Container already exists"
        delete_recreate = "Delete and recreate? [y/N]"
        deleting_old = "Deleting old container..."
        starting_existing = "Starting existing container..."
        creating_data_dir = "Creating data directory..."
        waiting_service = "Waiting for service to start..."
        installing_agents = "Installing Agent software..."
        agent_install_success = "Agent software installed successfully"
        gpu_not_supported = "Windows Docker Desktop does not support GPU passthrough"
    }
}

# ============================================================================
# Global state
# ============================================================================
$script:Lang = "cn"
$script:SelectedDesktop = "xfce"
$script:DockerMode = "none"
$script:Registry = ""
$script:SelectedImage = ""
$script:Version = "latest"
$script:DataDir = ""
$script:DesktopPort = 3001
$script:InstallAgents = @()
$script:AgentPorts = @{ openclaw = 18789; openfang = 4200; zeroclaw = 42617 }
$script:CustomAgentPorts = @{}
$script:ContainerName = "agent-workspace"
$script:RegistryCN = "registry.cn-hangzhou.aliyuncs.com/fliaping/agent-workspace"
$script:RegistryEN = "xuping/agent-workspace"
$script:SshPassword = ""
$script:SshPort = 2222
$script:DesktopUser = "agent"
$script:DesktopPassword = ""

# ============================================================================
# Helpers
# ============================================================================
function T($key) { return $Texts[$script:Lang][$key] }

function Write-Info { param($Message) Write-Host "[INFO] $Message" -ForegroundColor Cyan }
function Write-Success { param($Message) Write-Host "[OK] $Message" -ForegroundColor Green }
function Write-Warn { param($Message) Write-Host "[WARN] $Message" -ForegroundColor Yellow }
function Write-Err { param($Message) Write-Host "[ERROR] $Message" -ForegroundColor Red }

function Test-PortAvailable {
    param([int]$Port)
    try {
        $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $Port)
        $listener.Start()
        $listener.Stop()
        return $true
    } catch {
        return $false
    }
}

function Find-AvailablePort {
    param([int]$BasePort)
    $port = $BasePort
    while (-not (Test-PortAvailable $port)) {
        $port++
        if ($port -gt ($BasePort + 100)) {
            Write-Err "$(T 'port_in_use'): no available port found"
            exit 1
        }
    }
    return $port
}

# ============================================================================
# Docker checks
# ============================================================================
function Test-Docker {
    try {
        $v = docker --version 2>$null
        if ($v) { Write-Success "Docker: $v"; return $true }
    } catch {}
    return $false
}

function Test-DockerRunning {
    try {
        docker info 2>$null | Out-Null
        return $LASTEXITCODE -eq 0
    } catch {
        return $false
    }
}

# ============================================================================
# Step 1: Select language
# ============================================================================
function Select-Language {
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host "  $(T 'welcome_title')" -ForegroundColor Cyan
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""
    Write-Info (T 'step1_title')
    Write-Host "  $(T 'lang_cn')"
    Write-Host "  $(T 'lang_en')"
    Write-Host ""
    $choice = Read-Host "$(T 'enter_choice') [1-2, default 1]"
    if (-not $choice) { $choice = "1" }
    switch ($choice) {
        "2" { $script:Lang = "en"; Write-Info "Language: English" }
        default { $script:Lang = "cn"; Write-Info "Language: 中文" }
    }
}

# ============================================================================
# Step 2: Select desktop
# ============================================================================
function Select-Desktop {
    Write-Host ""
    Write-Info (T 'step_desktop_title')
    Write-Host "  $(T 'desktop_xfce')"
    Write-Host "  $(T 'desktop_lxqt')"
    Write-Host "  $(T 'desktop_kde')"
    Write-Host ""
    $choice = Read-Host "$(T 'enter_choice') [1-3, default 1]"
    if (-not $choice) { $choice = "1" }
    switch ($choice) {
        "2" { $script:SelectedDesktop = "lxqt" }
        "3" { $script:SelectedDesktop = "kde" }
        default { $script:SelectedDesktop = "xfce" }
    }
    Write-Host ""
    Write-Info "$(T 'selected_desktop'): $($script:SelectedDesktop)"
}

# ============================================================================
# Step 3: Docker mode
# ============================================================================
function Select-DockerMode {
    Write-Host ""
    Write-Info (T 'step_docker_title')
    Write-Host "  $(T 'docker_mode_none')"
    Write-Host "  $(T 'docker_mode_dind')"
    Write-Host "  $(T 'docker_mode_socket')"
    Write-Host ""
    $choice = Read-Host "$(T 'enter_choice') [1-3, default 1]"
    if (-not $choice) { $choice = "1" }
    switch ($choice) {
        "2" { $script:DockerMode = "dind" }
        "3" { $script:DockerMode = "socket" }
        default { $script:DockerMode = "none" }
    }
    $desc = switch ($script:DockerMode) {
        "dind"   { T 'docker_mode_dind_desc' }
        "socket" { T 'docker_mode_socket_desc' }
        default  { T 'docker_mode_none_desc' }
    }
    Write-Host ""
    Write-Info "$(T 'docker_mode_selected'): $desc"
}

# ============================================================================
# Step 4: Select registry
# ============================================================================
function Select-Registry {
    Write-Host ""
    Write-Info (T 'step_registry_title')
    Write-Host "  $(T 'registry_cn')"
    Write-Host "  $(T 'registry_en')"
    Write-Host ""
    $choice = Read-Host "$(T 'enter_choice') [1-2, default 1]"
    if (-not $choice) { $choice = "1" }
    switch ($choice) {
        "2" {
            $script:Registry = $script:RegistryEN
        }
        default {
            $script:Registry = $script:RegistryCN
        }
    }
    Write-Host ""
    Write-Info "$(T 'using_registry'): $($script:Registry)"
}

# ============================================================================
# Step 5: Select version
# ============================================================================
function Select-Version {
    Write-Host ""
    Write-Info (T 'step_version_title')
    Write-Host "  $(T 'version_latest')"
    Write-Host "  $(T 'version_custom')"
    Write-Host ""
    $choice = Read-Host "$(T 'enter_choice') [1-2, default 1]"
    if (-not $choice) { $choice = "1" }
    switch ($choice) {
        "2" {
            Write-Host ""
            $script:Version = Read-Host (T 'enter_version')
            if (-not $script:Version) { $script:Version = "latest" }
        }
        default { $script:Version = "latest" }
    }

    if ($script:Version -eq "latest") {
        $script:SelectedImage = "$($script:Registry):ubuntu-$($script:SelectedDesktop)"
    } else {
        $script:SelectedImage = "$($script:Registry):ubuntu-$($script:SelectedDesktop)-$($script:Version)"
    }
    Write-Host ""
    Write-Info "$(T 'selected_image'): $($script:SelectedImage)"
}

# ============================================================================
# Step 6: Data directory
# ============================================================================
function Select-DataDir {
    Write-Host ""
    Write-Info (T 'step_data_dir_title')
    $defaultDir = Join-Path $HOME "agent-workspace-data"
    Write-Host ""
    Write-Host "  1) $(T 'data_dir_default'): $defaultDir"
    Write-Host "  2) $(T 'data_dir_custom')"
    Write-Host ""
    $choice = Read-Host "$(T 'enter_choice') [1-2, default 1]"
    if (-not $choice) { $choice = "1" }
    switch ($choice) {
        "2" {
            Write-Host ""
            $custom = Read-Host (T 'enter_data_dir')
            if ($custom) { $script:DataDir = $custom } else { $script:DataDir = $defaultDir }
        }
        default { $script:DataDir = $defaultDir }
    }
    Write-Host ""
    Write-Info "$(T 'data_dir'): $($script:DataDir)"
}

# ============================================================================
# Step 7: Desktop port
# ============================================================================
function Select-DesktopPort {
    Write-Host ""
    Write-Info (T 'step_port_title')
    Write-Host ""
    Write-Info "$(T 'port_desktop'): $($script:DesktopPort)"
    Write-Host ""
    Write-Host "  1) $(T 'port_auto')"
    Write-Host "  2) $(T 'port_manual')"
    Write-Host ""
    $choice = Read-Host "$(T 'enter_choice') [1-2, default 1]"
    if (-not $choice) { $choice = "1" }
    switch ($choice) {
        "2" {
            Write-Host ""
            $custom = Read-Host "$(T 'enter_desktop_port') [default $($script:DesktopPort)]"
            if ($custom) {
                $port = [int]$custom
                if (Test-PortAvailable $port) {
                    $script:DesktopPort = $port
                    Write-Success "$(T 'port_desktop') $port $(T 'port_available')"
                } else {
                    Write-Warn "$(T 'port_desktop') $port $(T 'port_in_use')"
                    $script:DesktopPort = Find-AvailablePort $port
                    Write-Info "$(T 'auto_change_port'): $($script:DesktopPort)"
                }
            }
        }
        default {
            if (-not (Test-PortAvailable $script:DesktopPort)) {
                Write-Warn "$(T 'port_desktop') $($script:DesktopPort) $(T 'port_in_use')"
                $script:DesktopPort = Find-AvailablePort $script:DesktopPort
                Write-Info "$(T 'auto_change_port'): $($script:DesktopPort)"
            } else {
                Write-Success "$(T 'port_desktop') $($script:DesktopPort) $(T 'port_available')"
            }
        }
    }
    Write-Host ""
    Write-Info "$(T 'port_desktop'): $($script:DesktopPort)"
}

# ============================================================================
# SSH Configuration
# ============================================================================
function Configure-Ssh {
    Write-Host ""
    Write-Info (T 'ssh_password_prompt')
    Write-Host ""
    $sshPass = Read-Host (T 'ssh_password_prompt')
    if (-not $sshPass) {
        $script:SshPassword = ""
        return
    }

    $script:SshPassword = $sshPass

    Write-Host ""
    $customSshPort = Read-Host "$(T 'ssh_port') [default $($script:SshPort)]"
    if ($customSshPort) {
        $script:SshPort = [int]$customSshPort
    }

    # Check SSH port availability
    if (-not (Test-PortAvailable $script:SshPort)) {
        Write-Warn "$(T 'ssh_port') $($script:SshPort) $(T 'port_in_use')"
        $script:SshPort = Find-AvailablePort $script:SshPort
        Write-Info "$(T 'auto_change_port'): $($script:SshPort)"
    } else {
        Write-Success "$(T 'ssh_port') $($script:SshPort) $(T 'port_available')"
    }

    Write-Host ""
    Write-Info "$(T 'ssh_port'): $($script:SshPort)"
}

# ============================================================================
# Step 8: Select agents
# ============================================================================
function Select-Agents {
    Write-Host ""
    Write-Info (T 'step_agent_title')
    Write-Host ""
    Write-Info (T 'agent_install_title')
    Write-Host "  $(T 'agent_codex')"
    Write-Host "  $(T 'agent_claude_code')"
    Write-Host "  $(T 'agent_hermes')"
    Write-Host "  $(T 'agent_openclaw')"
    Write-Host "  $(T 'agent_openfang')"
    Write-Host "  $(T 'agent_zeroclaw')"
    Write-Host "  $(T 'agent_skip')"
    Write-Host ""
    $choices = Read-Host "$(T 'enter_agents') [1-7, default 1]"
    if (-not $choices) { $choices = "1" }

    $script:InstallAgents = @()
    foreach ($c in $choices.Split(" ", [StringSplitOptions]::RemoveEmptyEntries)) {
        switch ($c) {
            "1" { $script:InstallAgents += "codex" }
            "2" { $script:InstallAgents += "claude-code" }
            "3" { $script:InstallAgents += "hermes" }
            "4" { $script:InstallAgents += "openclaw" }
            "5" { $script:InstallAgents += "openfang" }
            "6" { $script:InstallAgents += "zeroclaw" }
            "7" { $script:InstallAgents = @(); return }
        }
    }

    if ($script:InstallAgents.Count -gt 0) {
        Write-Host ""
        Write-Info "Selected: $($script:InstallAgents -join ', ')"
    }
}

# ============================================================================
# Step 9: Configure agent ports
# ============================================================================
function Configure-AgentPorts {
    if ($script:InstallAgents.Count -eq 0) { return }
    $serviceAgents = @($script:InstallAgents | Where-Object { $script:AgentPorts.ContainsKey($_) })
    if ($serviceAgents.Count -eq 0) { return }
    Write-Host ""
    Write-Info (T 'step_agent_port_title')

    foreach ($agent in $serviceAgents) {
        $defaultPort = $script:AgentPorts[$agent]
        $portName = T "agent_port_$agent"
        Write-Host ""
        Write-Host "  $portName"
        Write-Host "  1) $(T 'agent_port_default'): $defaultPort"
        Write-Host "  2) $(T 'agent_port_custom')"
        Write-Host ""
        $choice = Read-Host "$(T 'enter_choice') [1-2, default 1]"
        if (-not $choice) { $choice = "1" }

        switch ($choice) {
            "2" {
                $custom = Read-Host (T 'enter_agent_port')
                if ($custom) {
                    $port = [int]$custom
                    if (Test-PortAvailable $port) {
                        $script:CustomAgentPorts[$agent] = $port
                        Write-Success "$portName $port $(T 'port_available')"
                    } else {
                        Write-Warn "$portName $port $(T 'port_in_use')"
                        $newPort = Find-AvailablePort $port
                        $script:CustomAgentPorts[$agent] = $newPort
                        Write-Info "$(T 'auto_change_port'): $newPort"
                    }
                } else {
                    $script:CustomAgentPorts[$agent] = $defaultPort
                }
            }
            default {
                if (Test-PortAvailable $defaultPort) {
                    $script:CustomAgentPorts[$agent] = $defaultPort
                    Write-Success "$portName $defaultPort $(T 'port_available')"
                } else {
                    Write-Warn "$portName $defaultPort $(T 'port_in_use')"
                    $newPort = Find-AvailablePort $defaultPort
                    $script:CustomAgentPorts[$agent] = $newPort
                    Write-Info "$(T 'auto_change_port'): $newPort"
                }
            }
        }
    }
}

# ============================================================================
# Install agents in container
# ============================================================================
function Install-AgentsInContainer {
    if ($script:InstallAgents.Count -eq 0) { return }

    $flags = "--non-interactive"
    if ($script:Lang -eq "cn") {
        $flags += " --china-mirror"
    }

    $agentArgs = $script:InstallAgents -join " "
    Write-Info "Installing agents: $agentArgs"

    # Install as the desktop user (abc); running as root leaves root-owned
    # files such as /config/.codex that the Agent cannot write at login.
    docker exec -u abc -e HOME=/config $script:ContainerName bash -c "install-agent.sh $flags $agentArgs"
    if ($LASTEXITCODE -ne 0) {
        Write-Warn "Agent installation failed, please install manually"
    }

    Write-Success (T 'agent_install_success')
}

# ============================================================================
# Print access info
# ============================================================================
function Print-AccessInfo {
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Green
    Write-Host "  $(T 'access_info')" -ForegroundColor Green
    Write-Host "========================================" -ForegroundColor Green
    Write-Host ""

    Write-Info "$(T 'desktop_url') (HTTPS): https://localhost:$($script:DesktopPort)/"
    if ($script:DesktopPassword) {
        Write-Info "Webtop user: $($script:DesktopUser)"
        Write-Info "Webtop password: $($script:DesktopPassword)"
    }
    Write-Info "$(T 'data_dir'): $($script:DataDir)"

    # SSH info
    if ($script:SshPassword) {
        Write-Info "SSH: ssh abc@localhost -p $($script:SshPort)"
    }

    $dockerDesc = switch ($script:DockerMode) {
        "dind"   { T 'docker_mode_dind_desc' }
        "socket" { T 'docker_mode_socket_desc' }
        default  { T 'docker_mode_none_desc' }
    }
    Write-Info "Docker: $dockerDesc"

    if ($script:InstallAgents.Count -gt 0) {
        Write-Host ""
        Write-Info "Agent software:"
        foreach ($agent in $script:InstallAgents) {
            switch ($agent) {
                "codex" { Write-Host "    codex - run codex to sign in" }
                "claude-code" { Write-Host "    claude-code - run claude to sign in" }
                "hermes" { Write-Host "    hermes - run hermes setup --portal" }
                default {
                    $cp = $script:CustomAgentPorts[$agent]
                    $ip = $script:AgentPorts[$agent]
                    Write-Host "    $agent (host:$cp -> container:$ip)"
                }
            }
        }
        Write-Host ""
        Write-Info "Workspace control: docker exec $($script:ContainerName) workspacectl info"
    }

    Write-Host ""
    Write-Info "$(T 'common_commands'):"
    Write-Host "    $(T 'view_logs'):    docker logs -f $($script:ContainerName)"
    Write-Host "    $(T 'stop_service'):  docker stop $($script:ContainerName)"
    Write-Host "    $(T 'start_service'): docker start $($script:ContainerName)"
    Write-Host "    $(T 'enter_container'): docker exec -it $($script:ContainerName) bash"
    Write-Host "========================================" -ForegroundColor Green
}

# ============================================================================
# Main
# ============================================================================
function Main {
    # Check Docker
    Write-Info (T 'check_docker')
    if (-not (Test-Docker)) {
        Write-Err (T 'docker_not_installed')
        Write-Info "$(T 'docker_install_guide'): https://www.docker.com/products/docker-desktop"
        exit 1
    }
    if (-not (Test-DockerRunning)) {
        Write-Err (T 'docker_daemon_not_running')
        Write-Info (T 'start_docker_service')
        exit 1
    }

    # GPU note
    Write-Warn (T 'gpu_not_supported')

    # Step 1-9
    Select-Language
    Select-Desktop
    Select-DockerMode
    Select-Registry
    Select-Version
    Select-DataDir
    Select-DesktopPort
    Configure-Ssh
    Select-Agents
    Configure-AgentPorts

    # Handle existing container
    $existing = docker ps -a --format "{{.Names}}" 2>$null | Where-Object { $_ -eq $script:ContainerName }
    if ($existing) {
        Write-Warn "$(T 'container_exists'): $($script:ContainerName)"
        $recreate = Read-Host (T 'delete_recreate')
        if ($recreate -eq "y" -or $recreate -eq "Y") {
            Write-Info (T 'deleting_old')
            docker stop $script:ContainerName 2>$null | Out-Null
            docker rm $script:ContainerName 2>$null | Out-Null
        } else {
            Write-Info (T 'starting_existing')
            docker start $script:ContainerName
            Write-Success (T 'run_success')
            Print-AccessInfo
            return
        }
    }

    # Create data dir
    Write-Info (T 'creating_data_dir')
    if (-not (Test-Path $script:DataDir)) {
        New-Item -ItemType Directory -Path $script:DataDir -Force | Out-Null
    }

    # Pull image
    Write-Info (T 'pulling')
    docker pull $script:SelectedImage
    if ($LASTEXITCODE -ne 0) {
        Write-Err (T 'pull_failed')
        exit 1
    }
    Write-Success (T 'pull_success')

    # Build docker run args
    $passwordBytes = New-Object byte[] 24
    $passwordRng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    $passwordRng.GetBytes($passwordBytes)
    $passwordRng.Dispose()
    $script:DesktopPassword = [Convert]::ToBase64String($passwordBytes)
    $dockerArgs = @(
        "run", "-d",
        "--name", $script:ContainerName,
        "--restart", "unless-stopped",
        "--shm-size", "2gb",
        "-e", "PUID=1000",
        "-e", "PGID=1000",
        "-e", "TZ=Asia/Shanghai",
        "-e", "LC_ALL=zh_CN.UTF-8",
        "-e", "CUSTOM_USER=$($script:DesktopUser)",
        "-e", "PASSWORD=$($script:DesktopPassword)",
        "-e", "NODE_OPTIONS=--max-old-space-size=2048",
        "-e", "SELKIES_ENABLE_RATE_CONTROL=true",
        "-e", "SELKIES_RATE_CONTROL_MODE=crf,cbr",
        "-e", "SELKIES_CONGESTION_CONTROL=false",
        "-e", "SELKIES_ENABLE_RESIZE=true",
        "-v", "$($script:DataDir):/config",
        "-p", "$($script:DesktopPort):3001"
    )

    # Docker mode
    switch ($script:DockerMode) {
        "dind" {
            $dockerArgs += @("--privileged", "-e", "START_DOCKER=true")
        }
        "socket" {
            # Windows Docker Desktop: use named pipe
            $dockerArgs += @("-v", "//var/run/docker.sock:/var/run/docker.sock", "-e", "START_DOCKER=false")
        }
        default {
            $dockerArgs += @("-e", "START_DOCKER=false")
        }
    }

    # SSH (enabled when password is set)
    if ($script:SshPassword) {
        $dockerArgs += @("-e", "SSH_PASSWORD=$($script:SshPassword)", "-p", "$($script:SshPort):22")
    }

    # Agent port mappings
    foreach ($agent in $script:InstallAgents) {
        if (-not $script:AgentPorts.ContainsKey($agent)) { continue }
        $cp = $script:CustomAgentPorts[$agent]
        $ip = $script:AgentPorts[$agent]
        $dockerArgs += @("-p", "${cp}:${ip}")
    }

    $dockerArgs += $script:SelectedImage

    # Start container
    Write-Info (T 'running')
    & docker @dockerArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Err (T 'run_failed')
        exit 1
    }

    # Wait for service
    Write-Info (T 'waiting_service')
    $maxWait = 120
    $waited = 0
    while ($waited -lt $maxWait) {
        # Port 3000 answers 401 when CUSTOM_USER/PASSWORD are set (always here),
        # so treat any HTTP response as ready, like the image HEALTHCHECK.
        $health = docker exec $script:ContainerName curl -s -o /dev/null -w '%{http_code}' http://localhost:3000/ 2>$null
        if ("$health" -match '^[234][0-9][0-9]$') { break }
        Write-Host "." -NoNewline
        Start-Sleep -Seconds 3
        $waited += 3
    }
    Write-Host ""

    # Install agents
    if ($script:InstallAgents.Count -gt 0) {
        Write-Host ""
        Write-Info (T 'installing_agents')
        Install-AgentsInContainer
    }

    Write-Success "$(T 'complete')!"
    Print-AccessInfo
}

# ============================================================================
# Entry point
# ============================================================================
Main
