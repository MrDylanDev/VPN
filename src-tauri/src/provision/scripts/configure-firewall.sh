#!/usr/bin/env bash
set -euo pipefail

# configure-firewall.sh — Configure UFW to allow WireGuard traffic
# Target: Ubuntu 24.04

# --- OS check ---
if [ ! -f /etc/os-release ]; then
    echo "ERROR: /etc/os-release not found — unsupported OS" >&2
    exit 1
fi

# shellcheck source=/dev/null
. /etc/os-release

if [ "${ID:-}" != "ubuntu" ] || [ "${VERSION_ID:-}" != "24.04" ]; then
    echo "ERROR: Expected Ubuntu 24.04, got ${ID:-unknown} ${VERSION_ID:-unknown}" >&2
    exit 1
fi

# --- Configure UFW ---
ufw allow 51820/udp comment 'WireGuard VPN'
ufw --force enable

echo "Firewall configured: port 51820/udp allowed."
