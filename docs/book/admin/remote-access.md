# Remote Access & Security

By default BirdNet-Behavior binds to `0.0.0.0:8502` — reachable from any device on your LAN. **Viewing the dashboard is open (no login); only the `/admin` panel requires a password**, which a fresh install sets for you. This page covers reaching the station from your network and from elsewhere, safely.

## On your local network

Out of the box the dashboard is already reachable from other devices. Browse to `http://<pi-ip>:8502` from any device on the network — find the Pi's address with `hostname -I`.

> The `0.0.0.0` default exposes the dashboard to **everyone on your network**. That's usually fine at home: the read-only views are open, and the `/admin` panel is protected by the auto-generated password (see below). On an untrusted network (a shared flat, public Wi-Fi), confirm `CADDY_PWD` is set and consider restricting the bind.

## Restrict to this device only

To make the dashboard reachable **only from the machine itself**, bind to loopback:

```dotenv
BIRDNET_LISTEN=127.0.0.1:8502
```

(Or answer "restrict to this device" in the interactive installer.) Then reach it remotely with an SSH tunnel (`ssh -L 8502:localhost:8502 pi@host`) or a VPN — see [the private-tunnel section](#a-safer-alternative-a-private-tunnel) below.

## Do NOT expose it directly to the internet

The built-in server speaks plain HTTP and has no TLS of its own. **Never** port-forward `8502` straight to the internet. Instead, put it behind a reverse proxy that terminates HTTPS and adds authentication.

## Reverse proxy with HTTPS

### Caddy (simplest — automatic HTTPS)

Caddy fetches and renews a Let's Encrypt certificate for you. A whole `Caddyfile` can be:

```caddyfile
birds.example.com {
    reverse_proxy 127.0.0.1:8502
    basic_auth {
        # generate the hash with: caddy hash-password
        birder $2a$14$....hashed-password....
    }
}
```

### nginx

```nginx
server {
    listen 443 ssl;
    server_name birds.example.com;
    # ssl_certificate / ssl_certificate_key from certbot or your CA

    location / {
        proxy_pass http://127.0.0.1:8502;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header Upgrade $http_upgrade;          # WebSocket (live feed,
        proxy_set_header Connection "upgrade";           # spectrogram, kiosk)
    }
    auth_basic "BirdNet-Behavior";
    auth_basic_user_file /etc/nginx/.htpasswd;            # htpasswd -c … birder
}
```

> **WebSocket matters here.** The live dashboard feed, the spectrogram, and kiosk mode all use WebSockets (`/api/v2/ws/detections` and `/api/v2/ws/spectrogram`). Make sure your proxy forwards the `Upgrade`/`Connection` headers (shown above) or those features will silently stall.

## Built-in HTTP Basic Auth

The binary enforces HTTP Basic Auth on the **`/admin` panel** itself — no proxy required — using the BirdNET-Pi `CADDY_PWD` convention. **Viewing the dashboard and the read-only `/api/v2/*` endpoints stay open; only `/admin*` (settings, audio config, software update, system controls, backups, migration) prompts for a password.**

A fresh install **auto-generates a strong password** (username `birdnet`), prints it once in the post-install summary, and stores it as `CADDY_PWD` in `birdnet.conf`. Change it any time via `birdnet.conf` or the environment (so it works under Docker too):

```dotenv
CADDY_USER=birder      # optional — defaults to "birdnet"
CADDY_PWD=a-long-random-password
```

After editing the config, restart the service (`sudo systemctl restart birdnet-behavior`). This is **still plain HTTP** — only rely on it behind TLS, or on a trusted LAN. **Clearing `CADDY_PWD` leaves `/admin` open** to anyone who can reach the dashboard; if the server binds to a non-loopback address (e.g. the default `0.0.0.0`) with no `CADDY_PWD` set, it logs a prominent warning at startup. The live-detection WebSocket and `/api/v2/health` are exempt from this auth (a browser can't attach Basic-auth headers to a WebSocket handshake), and are read-only and outside `/admin` in any case — restrict those at the network layer if you need to.

## Cross-origin requests (CORS)

By default the API allows **no** cross-origin reads — a website you happen to visit can't read your station's data over the LAN. If you serve a separate dashboard from a different origin, allow it explicitly:

```dotenv
BIRDNET_CORS_ALLOWED_ORIGINS=https://dashboard.example.com
```

State-changing requests are protected by a CSRF guard regardless of this setting.

## A safer alternative: a private tunnel

For remote access without opening any ports, a mesh VPN like **Tailscale** or **WireGuard** is the easiest secure option: install it on the Pi and your phone/laptop, and reach `http://<tailscale-ip>:8502` as if you were on the same LAN — encrypted, no public exposure, no certificates to manage.

## Checklist

- [ ] Keep `CADDY_PWD` set so `/admin` stays protected (a fresh install generates one) — don't clear it on a non-loopback bind.
- [ ] Restrict the bind to `127.0.0.1` (+ SSH/VPN) if you don't need LAN access.
- [ ] Never port-forward `8502` directly — always terminate TLS at a proxy or use a VPN for off-LAN access.
- [ ] Forward WebSocket upgrade headers in the proxy.
