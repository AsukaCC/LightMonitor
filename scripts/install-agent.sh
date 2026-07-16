#!/usr/bin/env bash
# LightMonitor agent installer
# Default: download binary from GitHub Releases; --server-url is for API only.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/<owner>/LightMonitor/main/scripts/install-agent.sh | \
#     sudo bash -s -- --server-url https://monitor.example.com --token <AGENT_TOKEN>
#
# Offline / private network (download binary from the LightMonitor server):
#   ... | sudo bash -s -- --server-url "$SERVER" --token "$TOKEN" --from-server
set -euo pipefail

SERVER_URL=""
TOKEN=""
VERSION="latest"
INSTALL_DIR="/opt/lightmonitor"
SERVICE_NAME="lightmonitor-agent"
BIN_NAME="lightmonitor-agent"
GITHUB_REPO="${LIGHTMONITOR_GITHUB_REPO:-AsukaCC/LightMonitor}"
ASSET_OVERRIDE=""

usage() {
  cat <<'EOF'
Install LightMonitor agent as a systemd service.

Required:
  --server-url URL     LightMonitor server base URL (no trailing slash)
  --token TOKEN        Per-host agent token from server DB / install flow

Optional:
  --version TAG        Release tag (default: latest)
  --repo owner/name    GitHub repo for binary download (default: detect or env)
  --asset NAME         Force asset filename
  --from-server        Download agent from $SERVER_URL/releases/ instead of GitHub
  -h, --help           Show help

Examples:
  # Default: download agent binary from GitHub Releases
  sudo bash install-agent.sh --server-url https://mon.example.com --token abc123
  # Offline / private network: download from the LightMonitor server
  sudo bash install-agent.sh --server-url http://10.0.0.1:8080 --token abc123 --from-server
EOF
}

FROM_SERVER=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --server-url) SERVER_URL="${2%/}"; shift 2 ;;
    --token) TOKEN="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --repo) GITHUB_REPO="$2"; shift 2 ;;
    --asset) ASSET_OVERRIDE="$2"; shift 2 ;;
    --from-server) FROM_SERVER=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage; exit 1 ;;
  esac
done

if [[ "$(id -u)" -ne 0 ]]; then
  echo "Please run as root (sudo)." >&2
  exit 1
fi

if [[ -z "$SERVER_URL" || -z "$TOKEN" ]]; then
  echo "--server-url and --token are required." >&2
  usage
  exit 1
fi

if ! command -v systemctl >/dev/null 2>&1; then
  echo "systemd is required." >&2
  exit 1
fi

arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) asset="lightmonitor-agent-linux-x86_64" ;;
  aarch64|arm64) asset="lightmonitor-agent-linux-aarch64" ;;
  *)
    echo "Unsupported architecture: $arch" >&2
    exit 1
    ;;
esac

if [[ -n "$ASSET_OVERRIDE" ]]; then
  asset="$ASSET_OVERRIDE"
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
bin_path="$tmpdir/$BIN_NAME"

download() {
  local url="$1"
  local out="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$out"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$out" "$url"
  else
    echo "curl or wget required" >&2
    exit 1
  fi
}

if [[ "$FROM_SERVER" -eq 1 ]]; then
  # Prefer arch-specific name, fall back to x86_64 name used by Docker image
  if ! download "$SERVER_URL/releases/$asset" "$bin_path" 2>/dev/null; then
    download "$SERVER_URL/releases/lightmonitor-agent-linux-x86_64" "$bin_path"
  fi
else
  if [[ "$VERSION" == "latest" ]]; then
    url="https://github.com/${GITHUB_REPO}/releases/latest/download/${asset}"
  else
    url="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}/${asset}"
  fi
  download "$url" "$bin_path"
fi

chmod +x "$bin_path"
mkdir -p "$INSTALL_DIR"
install -m 0755 "$bin_path" "$INSTALL_DIR/$BIN_NAME"

cat >"/etc/systemd/system/${SERVICE_NAME}.service" <<UNIT
[Unit]
Description=LightMonitor Agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=LIGHTMONITOR_SERVER_URL=${SERVER_URL}
Environment=LIGHTMONITOR_AGENT_TOKEN=${TOKEN}
Environment=LIGHTMONITOR_STATE_DIR=${INSTALL_DIR}
ExecStart=${INSTALL_DIR}/${BIN_NAME}
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable "$SERVICE_NAME" >/dev/null

# `enable --now` does not restart an already-running service after its token or
# server URL changes. Remove the old registration marker and force a restart.
rm -f "$INSTALL_DIR/agent-id"
systemctl restart "$SERVICE_NAME"

for _ in {1..15}; do
  if [[ -s "$INSTALL_DIR/agent-id" ]]; then
    echo "Agent installed and registered: $SERVICE_NAME -> $SERVER_URL"
    exit 0
  fi
  sleep 1
done

echo "Agent service was installed, but registration did not complete." >&2
systemctl --no-pager --full status "$SERVICE_NAME" >&2 || true
journalctl -u "$SERVICE_NAME" -n 20 --no-pager >&2 || true
exit 1
