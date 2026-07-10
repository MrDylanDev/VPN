#!/usr/bin/env bash
set -euo pipefail

# configure-sysctl.sh — Enable IP forwarding for WireGuard
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

# --- Enable IP forwarding ---
cat > /etc/sysctl.d/99-wireguard.conf << 'SYSCTL'
net.ipv4.ip_forward = 1
net.ipv6.conf.all.forwarding = 1
SYSCTL

sysctl -p /etc/sysctl.d/99-wireguard.conf

echo "IP forwarding enabled (v4 and v6)."
