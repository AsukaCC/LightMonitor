# LightMonitor

[中文](#中文) · [日本語](#日本語) · [English](#english)

---

## 中文

### 项目简介

LightMonitor 是轻量级服务器监控系统，由 Rust 服务端、主动上报 Agent 和 React 管理界面组成。

- `/`：公开监控页，仅展示脱敏后的主机状态和资源用量。
- `/admin`：管理主机、安装 Agent、调整采集频率、查看历史趋势和版本管理。
- CPU、内存、磁盘显示“已用 / 总量”和保留两位小数的占比。
- 浏览器通过单向 SSE 接收事件，不使用 WebSocket 双向连接。
- SSH 密码使用 AES-256-GCM 加密后写入 SQLite；加密密钥保存在数据卷的 `lightmonitor.key`。
- 支持 SSH 密码和容器内 SSH identity 文件登录。
- 管理员可检查 GitHub Releases，手动选择升级或回退到仍存在的任意版本。
- 界面支持中文、日文和英文切换，并保存浏览器语言偏好。

仓库：<https://github.com/AsukaCC/LightMonitor>

镜像：`ghcr.io/asukacc/lightmonitor:latest`

### 文档入口

| 文档 | 说明 |
|------|------|
| **[开发者指南](docs/DEVELOPER.md)** | 本地开发、分支建议、打 tag 自动发版 |
| 本 README | 试用部署、Docker 启动 |

开发请优先在 `dev` 分支进行；合并到 `main` 后，推送 `v*` 标签（如 `v1.0.1`）即可触发 GitHub Actions 自动打包并发布 Release / GHCR 镜像，无需本地再执行发版打包命令。详情见 [docs/DEVELOPER.md](docs/DEVELOPER.md)。

### 部署方法

要求：Docker 24+，建议 Docker Compose v2。生产环境必须修改管理员密码。

创建 `.env`：

```env
LIGHTMONITOR_PORT=8080
LIGHTMONITOR_ADMIN_USERNAME=admin
LIGHTMONITOR_ADMIN_PASSWORD=replace-with-a-strong-password
LIGHTMONITOR_PUBLIC_URL=
LIGHTMONITOR_GITHUB_REPO=AsukaCC/LightMonitor
LIGHTMONITOR_AGENT_VERSION=latest
```

方式一，直接拉取预构建镜像：

```bash
docker volume create lightmonitor-data
docker pull ghcr.io/asukacc/lightmonitor:latest
docker run -d \
  --name lightmonitor \
  --restart unless-stopped \
  --env-file .env \
  -p 8080:8080 \
  -e HOST=0.0.0.0 \
  -e PORT=8080 \
  -e LIGHTMONITOR_DATA_DIR=/app/data \
  -e LIGHTMONITOR_VERSIONS_DIR=/app/data/versions \
  -e LIGHTMONITOR_MANAGED_UPDATES=true \
  -v lightmonitor-data:/app/data \
  ghcr.io/asukacc/lightmonitor:latest
```

方式二，克隆源码后本地构建：

```bash
git clone https://github.com/AsukaCC/LightMonitor.git
cd LightMonitor
cp .env.example .env
# 编辑 .env 后启动
docker compose up -d --build
```

也可以使用仓库中的预构建镜像 Compose：

```bash
docker compose -f docker-compose.release.yml up -d --pull always
```

打开 `http://服务器IP:8080`，进入 `/admin` 添加远程主机并安装 Agent。使用 SSH identity 文件时，将密钥目录只读挂载到 `/root/.ssh`，再在安装窗口填写容器内路径，例如 `/root/.ssh/id_rsa`。

版本升级和回退位于“版本管理”。服务会校验 Release 包的 SHA-256、切换数据卷中的活动版本并自动重启；若新版本启动失败，启动器会恢复上一个版本。该功能要求容器能够访问 GitHub。

数据和凭据位于 `lightmonitor-data` 卷。备份时必须同时保留 `lightmonitor.db` 和 `lightmonitor.key`，丢失密钥后已保存的 SSH 密码无法恢复。

```bash
curl http://127.0.0.1:8080/api/health
docker logs -f lightmonitor
```

如果构建阶段访问 Docker Hub 出现 `EOF`，先执行 `docker pull rust:1.96-bookworm`、`docker pull node:24-bookworm` 和 `docker pull debian:bookworm-slim`，确认 Docker 代理/镜像源可用后重新运行构建。

---

## 日本語

### 概要

LightMonitor は、Rust サーバー、プッシュ型 Agent、React 管理画面で構成された軽量サーバー監視システムです。

- `/`：機密情報を除いたホスト状態とリソース使用量を表示する公開画面。
- `/admin`：ホスト管理、Agent 導入、収集間隔、履歴グラフ、バージョン管理。
- CPU、メモリ、ディスクは「使用量 / 総量」と小数点以下 2 桁の使用率を表示します。
- ブラウザー通知には一方向 SSE を使用し、WebSocket は使用しません。
- SSH パスワードは AES-256-GCM で暗号化して SQLite に保存します。鍵はデータボリュームの `lightmonitor.key` に保存されます。
- SSH パスワード認証と、コンテナ内の SSH Identity ファイル認証に対応します。
- GitHub Releases を確認し、管理者が任意の既存バージョンへ手動で更新・ロールバックできます。
- 中国語、日本語、英語を画面から切り替えられます。

リポジトリ：<https://github.com/AsukaCC/LightMonitor>

イメージ：`ghcr.io/asukacc/lightmonitor:latest`

### デプロイ

Docker 24+ と Docker Compose v2 を推奨します。本番環境では管理者パスワードを必ず変更してください。

`.env` を作成します：

```env
LIGHTMONITOR_PORT=8080
LIGHTMONITOR_ADMIN_USERNAME=admin
LIGHTMONITOR_ADMIN_PASSWORD=replace-with-a-strong-password
LIGHTMONITOR_PUBLIC_URL=
LIGHTMONITOR_GITHUB_REPO=AsukaCC/LightMonitor
LIGHTMONITOR_AGENT_VERSION=latest
```

方法 1、ビルド済み Docker イメージを直接起動：

```bash
docker volume create lightmonitor-data
docker pull ghcr.io/asukacc/lightmonitor:latest
docker run -d \
  --name lightmonitor \
  --restart unless-stopped \
  --env-file .env \
  -p 8080:8080 \
  -e HOST=0.0.0.0 \
  -e PORT=8080 \
  -e LIGHTMONITOR_DATA_DIR=/app/data \
  -e LIGHTMONITOR_VERSIONS_DIR=/app/data/versions \
  -e LIGHTMONITOR_MANAGED_UPDATES=true \
  -v lightmonitor-data:/app/data \
  ghcr.io/asukacc/lightmonitor:latest
```

方法 2、GitHub からクローンしてローカルビルド：

```bash
git clone https://github.com/AsukaCC/LightMonitor.git
cd LightMonitor
cp .env.example .env
# .env を編集して起動
docker compose up -d --build
```

リポジトリのビルド済みイメージ用 Compose も利用できます：

```bash
docker compose -f docker-compose.release.yml up -d --pull always
```

`http://サーバーIP:8080` を開き、`/admin` でリモートホストを追加して Agent を導入します。SSH Identity ファイルを使う場合は、鍵ディレクトリを `/root/.ssh` に読み取り専用でマウントし、インストール画面に `/root/.ssh/id_rsa` などのコンテナ内パスを入力します。

「バージョン管理」では Release パッケージの SHA-256 を検証して更新またはロールバックします。サービスは自動再起動し、新バージョンの起動に失敗した場合は以前のバージョンへ戻ります。この機能には GitHub への接続が必要です。

データは `lightmonitor-data` ボリュームにあります。バックアップ時は `lightmonitor.db` と `lightmonitor.key` の両方を保存してください。鍵を失うと保存済み SSH パスワードは復元できません。

```bash
curl http://127.0.0.1:8080/api/health
docker logs -f lightmonitor
```

Docker Hub へのアクセス中に `EOF` が発生した場合は、Docker のプロキシまたはミラー設定を確認し、ベースイメージを先に `docker pull` してから再度ビルドしてください。

---

## English

### Overview

LightMonitor is a lightweight server monitoring system built from a Rust service, push-based agents, and a React admin console.

- `/`: public monitor with sanitized host status and resource usage.
- `/admin`: host management, agent installation, collection intervals, history, and version management.
- CPU, memory, and disk show used / total values plus percentages with two decimal places.
- Browser events use one-way SSE instead of WebSocket connections.
- SSH passwords are encrypted with AES-256-GCM before being stored in SQLite. The key is stored as `lightmonitor.key` in the data volume.
- SSH password authentication and identity files mounted inside the container are supported.
- Admins can inspect GitHub Releases and manually update or roll back to any release that still exists.
- The interface supports Chinese, Japanese, and English with a persisted language preference.

Repository: <https://github.com/AsukaCC/LightMonitor>

Image: `ghcr.io/asukacc/lightmonitor:latest`

### Deployment

Docker 24+ and Docker Compose v2 are recommended. Always change the admin password in production.

Create `.env`:

```env
LIGHTMONITOR_PORT=8080
LIGHTMONITOR_ADMIN_USERNAME=admin
LIGHTMONITOR_ADMIN_PASSWORD=replace-with-a-strong-password
LIGHTMONITOR_PUBLIC_URL=
LIGHTMONITOR_GITHUB_REPO=AsukaCC/LightMonitor
LIGHTMONITOR_AGENT_VERSION=latest
```

Option 1, run the prebuilt Docker image directly:

```bash
docker volume create lightmonitor-data
docker pull ghcr.io/asukacc/lightmonitor:latest
docker run -d \
  --name lightmonitor \
  --restart unless-stopped \
  --env-file .env \
  -p 8080:8080 \
  -e HOST=0.0.0.0 \
  -e PORT=8080 \
  -e LIGHTMONITOR_DATA_DIR=/app/data \
  -e LIGHTMONITOR_VERSIONS_DIR=/app/data/versions \
  -e LIGHTMONITOR_MANAGED_UPDATES=true \
  -v lightmonitor-data:/app/data \
  ghcr.io/asukacc/lightmonitor:latest
```

Option 2, clone GitHub and build locally:

```bash
git clone https://github.com/AsukaCC/LightMonitor.git
cd LightMonitor
cp .env.example .env
# Edit .env, then start the service
docker compose up -d --build
```

The repository also includes a Compose file for the prebuilt image:

```bash
docker compose -f docker-compose.release.yml up -d --pull always
```

Open `http://SERVER_IP:8080`, then use `/admin` to add remote hosts and install agents. For an SSH identity file, mount the key directory read-only at `/root/.ssh` and enter its container path, such as `/root/.ssh/id_rsa`, in the installation dialog.

Version Management verifies each Release bundle with SHA-256, switches the active version in the data volume, and restarts the service. The launcher restores the previous version if the selected version fails to start. GitHub access is required for this feature.

Application data lives in the `lightmonitor-data` volume. Back up both `lightmonitor.db` and `lightmonitor.key`; saved SSH passwords cannot be recovered without the key.

```bash
curl http://127.0.0.1:8080/api/health
docker logs -f lightmonitor
```

If a Docker Hub request fails with `EOF` during a build, verify Docker proxy or registry mirror settings, pull the base images first, and rerun the build.

## License

[GPL-3.0](LICENSE)
