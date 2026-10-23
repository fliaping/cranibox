# LinuxServer Webtop + DinD + GPU 加速 + 完整开发环境
#
# 构建参数:
#   DESKTOP=xfce (默认) | lxqt | kde    — 桌面环境 (内存: xfce 800M / lxqt 300M / kde 1.1G)
#   USE_CHINA_MIRROR=false (默认，国际源) | true (国内源)
#
# 示例:
#   docker compose build --pull                                       # XFCE + 国际源
#   docker compose build --pull --build-arg DESKTOP=kde                 # KDE 桌面
#   docker compose build --pull --build-arg USE_CHINA_MIRROR=true       # 国内源

ARG DESKTOP=xfce
FROM python:3.12-slim AS herdr-builder
COPY scripts/install-herdr.py /install-herdr.py
RUN python /install-herdr.py --output-directory /herdr
FROM rust:bookworm AS workspace-rust-builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY apps/node-desktop/src-tauri/ ./apps/node-desktop/src-tauri/
COPY addons/control-center/index.en.html addons/control-center/index.zh-cn.html addons/control-center/web.js addons/control-center/web.css ./addons/control-center/
COPY addons/novnc/ ./addons/novnc/
RUN cargo build --release --locked -p cranibox

# Follow official Webtop updates; builds must pull to refresh this moving tag.
# Selkies and PulseAudio startup scripts are inherited from the official image.
FROM linuxserver/webtop:ubuntu-${DESKTOP}

# Virtual-space managers verify this before creating data or waiting for enrollment.
LABEL io.cranibox.space-protocol="1"

USER root

# ==========================================
# 构建参数（必须在 ENV 之前声明）
# ==========================================
# USE_CHINA_MIRROR: 控制安装脚本使用国内还是国际镜像源（默认国际）
ARG USE_CHINA_MIRROR=false

# ==========================================
# 环境变量配置
# ==========================================

# 构建时传入的变量（运行时由 docker-compose 设置）
ENV USE_CHINA_MIRROR=${USE_CHINA_MIRROR}

# Selkies 桌面流：默认使用恒定质量 CRF，并允许按浏览器窗口动态调整分辨率。
# 保留 CBR 作为弱网场景下的可选项；GCC 会强制自适应码率并降低动态画质，因此默认关闭。
# 上游 Webtop 默认是 X11（Xvfb）；PixelFlux 的原生 Computer Use 接口（PIXELFLUX_CU）
# 只在 Wayland 模式下启动，因此显式开启 Wayland。遇到兼容问题可用 PIXELFLUX_WAYLAND=false 回退 X11。
ENV SELKIES_ENABLE_RATE_CONTROL=true \
    SELKIES_RATE_CONTROL_MODE=crf,cbr \
    SELKIES_CONGESTION_CONTROL=false \
    SELKIES_ENABLE_RESIZE=true \
    XFCE_PANEL_SCALING=true \
    PIXELFLUX_WAYLAND=true \
    PIXELFLUX_CU=8764

# 工具版本
ENV GO_VERSION="go1.22.4"
ENV NODE_VERSION="22"

# 工具路径（指向 /config，LinuxServer 持久化目录）
ENV HOME=/config
ENV GOROOT=/usr/local/go
ENV GOPATH=/config/go
ENV CARGO_HOME=/config/.cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV NPM_CONFIG_PREFIX=/config/.npm-global
ENV PIP_CACHE_DIR=/config/.cache/pip
ENV UV_CACHE_DIR=/config/.cache/uv

# PATH（运行时路径，与镜像源无关）
ENV PATH=/config/bin:/config/.local/bin:/config/node/bin:$GOPATH/bin:$CARGO_HOME/bin:/usr/local/cargo/bin:$GOROOT/bin:$NPM_CONFIG_PREFIX/bin:/config/.linuxbrew/bin:/config/.linuxbrew/sbin:/home/linuxbrew/.linuxbrew/bin:/home/linuxbrew/.linuxbrew/sbin:$PATH

# Homebrew 基础配置（镜像源地址由 install-tools.sh 写入 /etc/profile.d/mirrors.sh）
ENV HOMEBREW_NO_AUTO_UPDATE=1

# ==========================================
# 安装系统依赖
# ==========================================

# 国内模式切换 APT 源（USTC 镜像，加速 apt-get）。
# 同时兼容旧式 sources.list 和新版 Ubuntu 的 deb822 *.sources 文件。
RUN if [ "$USE_CHINA_MIRROR" = "true" ]; then \
        for apt_source_file in /etc/apt/sources.list /etc/apt/sources.list.d/*.sources; do \
            [ -f "$apt_source_file" ] || continue; \
            sed -i \
                -e 's@http://ports.ubuntu.com/ubuntu-ports@https://mirrors.ustc.edu.cn/ubuntu-ports@g' \
                -e 's@http://archive.ubuntu.com/ubuntu@https://mirrors.ustc.edu.cn/ubuntu@g' \
                -e 's@http://security.ubuntu.com/ubuntu@https://mirrors.ustc.edu.cn/ubuntu@g' \
                "$apt_source_file"; \
        done; \
    fi

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl gnupg sudo \
    build-essential git wget jq unzip xz-utils \
    python3 python3-pip python3-venv python3-dev zenity \
    libssl-dev libffi-dev lsof tmux wlrctl \
    openssh-server \
    locales fonts-wqy-zenhei fonts-wqy-microhei fonts-noto-cjk \
    && apt-get clean && rm -rf /var/lib/apt/lists/*

# ==========================================
# 安装开发工具
# 通过 install-tools.sh 统一管理，支持国内/国际两种模式
# ==========================================

# 构建时需要 /config 目录存在（运行时通常由 volume 挂载覆盖）
RUN mkdir -p /config/custom-services.d /config/agent-workspace-manager \
    && chown -R 1000:1000 /config

COPY scripts/install-tools.sh /usr/local/bin/install-tools.sh
RUN chmod +x /usr/local/bin/install-tools.sh \
    && USE_CHINA_MIRROR=${USE_CHINA_MIRROR} \
       GO_VERSION=${GO_VERSION} \
       NODE_VERSION=${NODE_VERSION} \
       /usr/local/bin/install-tools.sh

# Application-layer TUI framework. The image keeps only stable runtime
# dependencies; application logic is installed/updated under /config by
# agent-workspace-manager.
RUN pip3 install --no-cache-dir textual --break-system-packages \
    || pip3 install --no-cache-dir textual

# ==========================================
# 复制脚本和服务
# ==========================================

# Runtime scripts run as both root and abc. Do not inherit a private checkout's
# permissions: chmod +x alone still leaves scripts unreadable to abc.
COPY --chmod=0755 scripts/ /usr/local/bin/
COPY --chmod=0755 services/ /etc/services.d/
COPY --chmod=0755 agent-context/ /usr/local/share/agent-workspace/context/
COPY --chmod=0755 s6-rc.d/ /etc/s6-overlay/s6-rc.d/
COPY --chmod=0644 nginx/cranibox-applications.conf /usr/local/share/agent-workspace/cranibox-applications.conf
RUN cat /usr/local/share/agent-workspace/cranibox-applications.conf >> /defaults/default.conf
EXPOSE 3002
COPY --chmod=0755 --from=workspace-rust-builder /build/target/release/cranibox /usr/local/lib/agent-workspace/
# One program: the node CLI is the same binary under its historical name.
RUN ln -sf cranibox /usr/local/lib/agent-workspace/cranibox-node
COPY --chmod=0755 --from=herdr-builder /herdr/herdr /usr/local/lib/agent-workspace/herdr
# Startup tasks are native s6-rc oneshots; they leave no sleep process behind.
RUN mkdir -p /usr/local/lib/agent-workspace \
    && for task in init custom-services systemctl-services; do \
        cp "/etc/services.d/$task/run" "/usr/local/lib/agent-workspace/$task"; \
        chmod +x "/usr/local/lib/agent-workspace/$task"; \
        rm -rf "/etc/services.d/$task"; \
    done

# LinuxServer custom-init
RUN mkdir -p /custom-cont-init.d \
    && ln -sf /usr/local/bin/fix-locale.sh /custom-cont-init.d/fix-locale.sh \
    && ln -sf /usr/local/bin/fix-docker-tmpdir.sh /custom-cont-init.d/fix-docker-tmpdir.sh \
    && ln -sf /usr/local/bin/tailscale-authkey-init.sh /custom-cont-init.d/tailscale-authkey-init.sh

# exo-open and the XFCE preferred-application helper only exist on the XFCE
# base; LXQt and KDE resolve the browser through chromium.desktop.
RUN chmod +x /usr/local/bin/*.sh /usr/local/bin/agent-workspace-manager \
        /usr/local/bin/workspacectl /usr/local/bin/agent-workspace-browser \
        /usr/local/bin/agent-workspace-exo-open \
    && chmod a+r /usr/local/bin/workspacectl-*.py \
    && if [ -e /usr/bin/exo-open ]; then \
        mv /usr/bin/exo-open /usr/bin/exo-open-real \
        && cp /usr/local/bin/agent-workspace-exo-open /usr/bin/exo-open; \
    fi \
    && sed -i 's#^Exec=/usr/local/bin/wrapped-chromium.*#Exec=/usr/local/bin/agent-workspace-browser %U#' \
        /usr/share/applications/chromium.desktop \
    && grep -q '^Exec=/usr/local/bin/agent-workspace-browser' /usr/share/applications/chromium.desktop \
    && if [ -f /usr/share/xfce4/helpers/chromium.desktop ]; then \
        sed -i \
            -e 's#^X-XFCE-Binaries=.*#X-XFCE-Binaries=agent-workspace-browser;#' \
            -e 's#^X-XFCE-Commands=.*#X-XFCE-Commands=/usr/local/bin/agent-workspace-browser;#' \
            -e 's#^X-XFCE-CommandsWithParameter=.*#X-XFCE-CommandsWithParameter=/usr/local/bin/agent-workspace-browser "%s";#' \
            /usr/share/xfce4/helpers/chromium.desktop; \
    fi \
    && find /etc/services.d -name "run" -exec chmod +x {} \;

# systemctl wrapper: adds --user support on top of docker-systemctl-replacement
# user-systemctl.sh delegates system calls to /usr/bin/systemctl.py and
# execs the persistent Rust user manager, with a bootstrap fallback.
RUN cp /usr/local/bin/user-systemctl.sh /usr/local/bin/systemctl \
    && chmod +x /usr/local/bin/systemctl

# ==========================================
# 权限配置
# ==========================================

RUN echo "abc ALL=(ALL) NOPASSWD: ALL" > /etc/sudoers.d/abc \
    && chmod 0440 /etc/sudoers.d/abc \
    && usermod -aG docker abc

# SSH server: host keys persist in /config/.ssh-host-keys
RUN mkdir -p /run/sshd \
    && sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication yes/' /etc/ssh/sshd_config \
    && sed -i 's/^#\?PermitRootLogin.*/PermitRootLogin no/' /etc/ssh/sshd_config \
    && echo "HostKey /config/.ssh-host-keys/ssh_host_rsa_key" >> /etc/ssh/sshd_config \
    && echo "HostKey /config/.ssh-host-keys/ssh_host_ed25519_key" >> /etc/ssh/sshd_config

# ==========================================
# 健康检查
# ==========================================

HEALTHCHECK --interval=30s --timeout=10s --start-period=120s --retries=3 \
    CMD curl -sS -o /dev/null -w '%{http_code}' http://localhost:3000/ \
        | grep -Eq '^[234][0-9]{2}$'

# 使用 LinuxServer 原生的 /init 入口
