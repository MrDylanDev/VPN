#!/usr/bin/env bash
set -euo pipefail

# configure-dns.sh — Set DNS resolvers on the VPS
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

# --- Set DNS resolvers ---
cat > /etc/resolv.conf << 'RESOLV'
nameserver 1.1.1.1
nameserver 1.0.0.1
RESOLV

echo "DNS configured: 1.1.1.1, 1.0.0.1."
