# GitHub Actions Secrets 配置

本文档说明如何配置 GitHub Actions 所需的密钥，以实现 Docker 镜像的自动构建和推送。

## 需要配置的 Secrets

### Docker Hub 密钥

| Secret 名称 | 说明 | 获取方式 |
|------------|------|----------|
| `DOCKERHUB_USERNAME` | Docker Hub 用户名 | https://hub.docker.com/ |
| `DOCKERHUB_TOKEN` | Docker Hub 访问令牌 | https://hub.docker.com/settings/security |

### 阿里云容器镜像服务密钥

| Secret 名称 | 说明 | 获取方式 |
|------------|------|----------|
| `ALIYUN_REGISTRY_USERNAME` | 阿里云账号（通常是邮箱） | 阿里云控制台 |
| `ALIYUN_REGISTRY_PASSWORD` | 阿里云容器镜像服务密码 | 容器镜像服务控制台 |

### npm 发布（不需要密钥）

npm 包经 Trusted Publishing 发布：GitHub Actions 用 OIDC 身份直接发布，不需要 `NPM_TOKEN`。
七个包都已在 npmjs.com 配置 Trusted Publisher（仓库 `fliaping/cranibox`、工作流 `rust-runtime.yml`、环境 `npm`）。
`NPM_TOKEN` 只在正式版仍用暂存审批（`CRANIBOX_NPM_PUBLISH_MODE=stage`）或手动补发时使用。详见 [客户端分发](../docs/distribution.md)。

## 配置步骤

### 1. 配置 Docker Hub Secrets

1. 访问 https://hub.docker.com/settings/security
2. 点击 "New Access Token"
3. 输入令牌名称（如 "GitHub Actions"）
4. 选择权限：
   - `read` - 读取镜像
   - `write` - 推送镜像
   - `delete` - 删除镜像（可选）
5. 点击 "Generate" 并复制令牌

### 2. 配置阿里云 Secrets

1. 登录阿里云控制台：https://cr.console.aliyun.com/
2. 进入「容器镜像服务」→「个人实例」或「企业实例」
3. 在「访问凭证」中设置固定密码
4. 记录：
   - 用户名：通常是阿里云账号邮箱
   - 密码：刚才设置的固定密码

### 3. 在 GitHub 仓库中配置 Secrets

1. 打开 GitHub 仓库页面
2. 点击 Settings → Secrets and variables → Actions
3. 点击 "New repository secret"
4. 依次添加以下 secrets：
   - `DOCKERHUB_USERNAME`
   - `DOCKERHUB_TOKEN`
   - `ALIYUN_REGISTRY_USERNAME`
   - `ALIYUN_REGISTRY_PASSWORD`

## 镜像仓库地址

### Docker Hub（英文镜像）
- 仓库地址：`https://hub.docker.com/r/xuping/agent-workspace`
- 镜像名称：`xuping/agent-workspace`
- 使用 Dockerfile：`Dockerfile`（`DESKTOP` 构建参数选择桌面）

### 阿里云容器镜像服务（中文镜像）
- 仓库地址：`https://cr.console.aliyun.com/`
- 镜像名称：`registry.cn-hangzhou.aliyuncs.com/fliaping/agent-workspace`
- 使用 Dockerfile：`Dockerfile`（与 Docker Hub 相同的构建，`DESKTOP` 构建参数选择桌面）

## 触发构建

### 自动触发
- 推送到 `main` 分支
- 创建版本标签（如 `v1.0.0`）
- 发起 Pull Request

### 手动触发
1. 进入 GitHub 仓库 Actions 页面
2. 选择 "Build and Push Docker Images" 工作流
3. 点击 "Run workflow"
4. 输入版本号（可选，默认为 latest）

## 构建流程

```
推送代码/标签 ──→ GitHub Actions 触发
                      │
        ┌─────────────┼─────────────┐
        ↓             ↓             ↓
   Docker Hub    阿里云镜像      并行构建
   (英文镜像)    (中文镜像)
        │             │
        └─────────────┘
              ↓
        镜像推送完成
```

## 镜像标签规则

| 触发条件 | Docker Hub 标签 | 阿里云标签 |
|---------|----------------|-----------|
| 推送到 main | `latest` | `latest` |
| 推送标签 v1.0.0 | `v1.0.0`, `1.0` | `v1.0.0`, `1.0` |
| 手动触发 v2.0.0 | `v2.0.0` | `v2.0.0` |
| Pull Request | 不推送 | 不推送 |

## 故障排查

### 构建失败
1. 检查 Secrets 是否正确配置
2. 检查 Dockerfile 是否有语法错误
3. 查看 Actions 日志获取详细错误信息

### 推送失败
1. 检查 Docker Hub/阿里云账号是否有推送权限
2. 检查仓库是否已创建
3. 检查网络连接

### 镜像未更新
1. 检查是否正确触发工作流
2. 检查缓存是否命中（可能需要清除缓存重建）
