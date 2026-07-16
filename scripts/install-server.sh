#!/usr/bin/env bash
# Install LightMonitor server binary + systemd unit (non-Docker).
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/<owner>/LightMonitor/main/scripts/install-server.sh | \
#     sudo bash -s -- --repo <owner>/LightMonitor
set -euo pipefail

VERSION="latest"
GITHUB_REPO="${LIGHTMONITOR_GITHUB_REPO:-AsukaCC/LightMonitor}"
INSTALL_DIR="/opt/lightmonitor"
DATA_DIR="/var/lib/lightmonitor"
SERVICE_NAME="lightmonitor"
PORT="8080"
PUBLIC_URL=""
ADMIN_USER="admin"
ADMIN_PASS=""

usage() {
  cat <<'EOF'
Install LightMonitor server as systemd service.

Optional:
  --repo owner/name       GitHub repo (required unless LIGHTMONITOR_GITHUB_REPO)
  --version TAG           Release tag (default: latest)
  --port PORT             Listen port (default: 8080)
  --public-url URL        Override the auto-detected agent callback URL
  --admin-user NAME       Admin username (default: admin)
  --admin-password PASS   Admin password (random if omitted)
  -h, --help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) GITHUB_REPO="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --public-url) PUBLIC_URL="$2"; shift 2 ;;
    --admin-user) ADMIN_USER="$2"; shift 2 ;;
    --admin-password) ADMIN_PASS="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage; exit 1 ;;
  esac
done

if [[ "$(id -u)" -ne 0 ]]; then
  echo "Please run as root (sudo)." >&2
  exit 1
fi

if [[ -z "$ADMIN_PASS" ]]; then
  if command -v openssl >/dev/null 2>&1; then
    ADMIN_PASS="$(openssl rand -hex 12)"
  else
    ADMIN_PASS="change-me-$(date +%s)"
  fi
fi

arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) asset="lightmonitor-server-linux-x86_64" ;;
  aarch64|arm64) asset="lightmonitor-server-linux-aarch64" ;;
  *) echo "Unsupported arch: $arch" >&2; exit 1 ;;
esac

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

if [[ "$VERSION" == "latest" ]]; then
  url="https://github.com/${GITHUB_REPO}/releases/latest/download/${asset}"
else
  url="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/${asset}"
fi

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$tmpdir/server"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$tmpdir/server" "$url"
else
  echo "curl or wget required" >&2
  exit 1
fi

chmod +x "$tmpdir/server"
mkdir -p "$INSTALL_DIR/web" "$INSTALL_DIR/releases" "$DATA_DIR"
install -m 0755 "$tmpdir/server" "$INSTALL_DIR/lightmonitor-server"

# Prefer agent asset matching this arch for /releases (SSH install / --from-server)
agent_asset="lightmonitor-agent-linux-x86_64"
case "$arch" in
  aarch64|arm64) agent_asset="lightmonitor-agent-linux-aarch64" ;;
esac
if [[ "$VERSION" == "latest" ]]; then
  agent_url="https://github.com/${GITHUB_REPO}/releases/latest/download/${agent_asset}"
else
  agent_url="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/${agent_asset}"
fi
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$agent_url" -o "$INSTALL_DIR/releases/lightmonitor-agent-linux-x86_64" || true
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$INSTALL_DIR/releases/lightmonitor-agent-linux-x86_64" "$agent_url" || true
fi
chmod +x "$INSTALL_DIR/releases/"* 2>/dev/null || true

# Minimal placeholder web if not bundled in binary release
if [[ ! -f "$INSTALL_DIR/web/index.html" ]]; then
  cat >"$INSTALL_DIR/web/index.html" <<'HTML'
<!doctype html>
<html><head><meta charset="utf-8"><title>LightMonitor</title></head>
<body>
  <p>LightMonitor server is running (API only). Ship web/dist with Docker image for full UI, or place SPA files in LIGHTMONITOR_WEB_DIR.</p>
</body></html>
HTML
fi

env_file="/etc/lightmonitor.env"
cat >"$env_file" <<ENV
HOST=0.0.0.0
PORT=${PORT}
LIGHTMONITOR_DATA_DIR=${DATA_DIR}
LIGHTMONITOR_WEB_DIR=${INSTALL_DIR}/web
LIGHTMONITOR_RELEASES_DIR=${INSTALL_DIR}/releases
LIGHTMONITOR_PUBLIC_URL=${PUBLIC_URL}
LIGHTMONITOR_ADMIN_USERNAME=${ADMIN_USER}
LIGHTMONITOR_ADMIN_PASSWORD=${ADMIN_PASS}
ENV
chmod 600 "$env_file"

cat >"/etc/systemd/system/${SERVICE_NAME}.service" <<UNIT
[Unit]
Description=LightMonitor Server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
EnvironmentFile=${env_file}
ExecStart=${INSTALL_DIR}/lightmonitor-server
Restart=always
RestartSec=3
WorkingDirectory=${INSTALL_DIR}

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable --now "$SERVICE_NAME"

if [[ -n "$PUBLIC_URL" ]]; then
  echo "Server installed: ${PUBLIC_URL} (env ${env_file})"
else
  echo "Server installed on port ${PORT}; public URL will follow the domain used to open the admin UI (env ${env_file})"
fi
