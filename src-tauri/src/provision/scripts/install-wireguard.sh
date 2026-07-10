#!/usr/bin/env bash
set -euo pipefail

# install-wireguard.sh — Install WireGuard and generate server keys
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

# --- Install WireGuard ---
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq wireguard

# --- Generate server keypair ---
umask 077
wg genkey | tee /etc/wireguard/server.key | wg pubkey > /etc/wireguard/server.pub
umask 022

echo "WireGuard installed and server keypair generated."
