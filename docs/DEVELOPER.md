# LightMonitor 开发者指南

面向本地开发、联调与发版的维护者。试用与部署见根目录 [README.md](../README.md)。

## 1. 仓库结构

```text
LightMonitor/
├── crates/
│   ├── server/          # HTTP API、SSE、SQLite、SSH 安装
│   └── agent/           # 探针，定时采集并上报
├── web/                 # React 管理台 / 公开页（Vite）
├── scripts/             # 安装与启动脚本
├── .github/workflows/
│   ├── ci.yml           # push / PR 时检查与前端构建
│   └── release.yml      # 推送 v* tag 后自动发版
└── docker-compose*.yml
```

## 2. 本地开发

### 环境要求

- Rust（stable，与 `Cargo.toml` / CI 一致）
- Node.js 24+、npm
- 可选：Docker 24+（本地容器构建）

### 后端

```bash
# 在仓库根目录
cargo run -p server
```

常用环境变量见 `.env.example`。本地可先复制：

```bash
cp .env.example .env
```

健康检查：

```bash
curl http://127.0.0.1:8080/api/health
```

### 前端

```bash
cd web
npm ci
npm run dev
```

生产构建：

```bash
cd web
npm run build
```

默认前端开发服务器会通过 Vite 代理访问后端 API（见 `web/vite.config.ts`）。

### 探针（Agent）

```bash
cargo run -p agent
```

需配置服务端地址与认证 Token（安装探针时由管理台生成）。

### Docker 本地构建

```bash
cp .env.example .env
# 编辑管理员密码等
docker compose up -d --build
```

使用预构建镜像（不本地编译）：

```bash
docker compose -f docker-compose.release.yml up -d --pull always
```

## 3. 分支建议

| 分支 | 用途 |
|------|------|
| `main` | 稳定发布线，与线上一致 |
| `dev` | 日常开发与联调，合并进 `main` 后再打 tag 发版 |

推荐流程：

1. 在 `dev`（或功能分支）开发并推送  
2. 开 PR 合并到 `main`  
3. 在 `main` 上打版本 tag 触发自动发版  

## 4. 发版方法（自动打包）

**不需要在本地手动执行打包发版命令。**  
推送符合 `v*` 的 Git tag 后，GitHub Actions（`.github/workflows/release.yml`）会自动：

1. 编译多平台 `server` / `agent` 二进制  
2. 构建并推送镜像到 `ghcr.io/asukacc/lightmonitor`  
3. 创建 GitHub Release（附件 + `SHA256SUMS.txt` + 更新说明）

### 维护者步骤

确认待发代码已在 `main`（或你指定的发布提交）上：

```bash
git checkout main
git pull origin main

# 版本号与 Cargo.toml / web/package.json 保持一致更佳
git tag v1.0.4
git push origin v1.0.4
```

然后在仓库 **Actions → Release** 查看工作流是否成功。

产物位置：

| 产物 | 地址 |
|------|------|
| Release 附件 | https://github.com/AsukaCC/LightMonitor/releases |
| 容器镜像 | `ghcr.io/asukacc/lightmonitor:<去掉 v 的版本号>`、`:latest` |

说明：

- 仅 `git push` 到分支 **不会** 发版；必须推送 `v*` tag（如 `v1.0.4`）。
- `workflow_dispatch` 可在 Actions 里手动触发工作流，但正式发布 Release / 推镜像仍以 **推送 tag** 为准。  
- 镜像标签会去掉 tag 的 `v` 前缀：`v1.0.4` → 镜像 `1.0.4`。

### 发版后验证

```bash
docker pull ghcr.io/asukacc/lightmonitor:latest
# 或指定版本
docker pull ghcr.io/asukacc/lightmonitor:1.0.4
```

管理台「版本管理」可从 GitHub Releases 选择升级 / 回退（需容器能访问 GitHub）。

## 5. CI

| 工作流 | 触发 | 作用 |
|--------|------|------|
| `CI` | `main` / `master` 的 push，以及 PR | `cargo check` + 前端 `npm run build` |
| `Release` | 推送 `v*` tag | 二进制、镜像、GitHub Release |

## 6. 相关链接

- 试用与部署：根目录 [README.md](../README.md)  
- API：若仓库中有 [API.md](./API.md) 则以该文件为准  
- Issues / PR：https://github.com/AsukaCC/LightMonitor  
