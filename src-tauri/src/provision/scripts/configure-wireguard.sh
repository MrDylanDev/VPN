#!/usr/bin/env bash
set -euo pipefail

# configure-wireguard.sh — Create wg0.conf from /tmp/client.pub and start WireGuard
# Requires /tmp/client.pub to exist with the client's WireGuard public key

CLIENT_PUB=$(cat /tmp/client.pub)

cat > /etc/wireguard/wg0.conf <<EOF
[Interface]
Address = 10.0.0.1/24
ListenPort = 51820
PrivateKey = $(cat /etc/wireguard/server.key)

[Peer]
PublicKey = ${CLIENT_PUB}
AllowedIPs = 0.0.0.0/0, ::/0
EOF

systemctl enable wg-quick@wg0
systemctl start wg-quick@wg0

echo "WG_OK"
