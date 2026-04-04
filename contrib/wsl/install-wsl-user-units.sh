#!/usr/bin/env bash
# Install user systemd units for bdstorage on Ubuntu (WSL2 with systemd enabled).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CONFIG_DIR="${HOME}/.config/bdstorage"
SYSTEMD_USER="${HOME}/.config/systemd/user"

usage() {
  echo "Usage: $0 [--binary PATH] [--target PATH] [--interval SECS]"
  echo "  Creates ~/.config/bdstorage/env and installs user units under ~/.config/systemd/user/"
  exit 1
}

BIN_DEFAULT="${REPO_ROOT}/target/release/bdstorage"
BIN="${BIN_DEFAULT}"
TARGET="${BDSTORAGE_TARGET:-${HOME}/bdstorage-target}"
INTERVAL="${BDSTORAGE_INTERVAL_SECS:-3600}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary)
      BIN="${2:?}"
      shift 2
      ;;
    --target)
      TARGET="${2:?}"
      shift 2
      ;;
    --interval)
      INTERVAL="${2:?}"
      shift 2
      ;;
    -h|--help)
      usage
      ;;
    *)
      usage
      ;;
  esac
done

if [[ ! -x "$BIN" ]]; then
  echo "error: no executable at: $BIN"
  echo "  Build first: (cd \"$REPO_ROOT\" && cargo build --release)"
  echo "  Or pass: $0 --binary /path/to/bdstorage"
  exit 1
fi

mkdir -p "$CONFIG_DIR" "$SYSTEMD_USER"

ENV_FILE="${CONFIG_DIR}/env"
if [[ -f "$ENV_FILE" ]]; then
  echo "Keeping existing $ENV_FILE (remove it to regenerate)"
else
  umask 077
  cat >"$ENV_FILE" <<EOF
HOME=${HOME}
BDSTORAGE_BIN=${BIN}
BDSTORAGE_TARGET=${TARGET}
BDSTORAGE_INTERVAL_SECS=${INTERVAL}
EOF
  echo "Created $ENV_FILE"
fi

install -m0644 "$SCRIPT_DIR/bdstorage-dedupe.service" "$SYSTEMD_USER/"
install -m0644 "$SCRIPT_DIR/bdstorage-dedupe-once.service" "$SYSTEMD_USER/"
install -m0644 "$SCRIPT_DIR/bdstorage-dedupe.timer" "$SYSTEMD_USER/"

if ! command -v systemctl >/dev/null 2>&1; then
  echo "warning: systemctl not found — enable systemd in WSL (see contrib/wsl/README.md)"
  exit 0
fi

systemctl --user daemon-reload

echo ""
echo "Installed user units. Edit targets if needed: ${ENV_FILE}"
echo ""
echo "Long-running daemon (repeated dedupe every BDSTORAGE_INTERVAL_SECS):"
echo "  systemctl --user enable --now bdstorage-dedupe.service"
echo "  systemctl --user status bdstorage-dedupe.service"
echo ""
echo "Or timer (daily dedupe, no background loop):"
echo "  systemctl --user disable --now bdstorage-dedupe.service 2>/dev/null || true"
echo "  systemctl --user enable --now bdstorage-dedupe.timer"
echo "  systemctl --user list-timers bdstorage-dedupe.timer"
echo ""
echo "Optional: start user services before first login after WSL boot"
echo "  loginctl enable-linger \"${USER}\""
echo ""
