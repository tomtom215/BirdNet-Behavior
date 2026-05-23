# Remote Access & Security

By default BirdNet-Behavior binds to `127.0.0.1:8502` — reachable only from the machine itself. This page covers reaching it from elsewhere, safely.

## On your local network

To reach the dashboard from another device on your LAN, bind to all interfaces:

```dotenv
BIRDNET_LISTEN=0.0.0.0:8502
```

Then browse to `http://<pi-ip>:8502` from any device on the network. Find the Pi's address with `hostname -I`.

> Binding to `0.0.0.0` exposes the dashboard to **everyone on your network**. That's usually fine at home, but don't do it on an untrusted network (a shared flat, a public Wi-Fi) without the password protection below.

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

> **WebSocket matters here.** The live dashboard feed, the spectrogram, and kiosk mode all use a WebSocket (`/api/v2/ws`). Make sure your proxy forwards the `Upgrade`/`Connection` headers (shown above) or those features will silently stall.

## Built-in HTTP Basic Auth

If you'd rather not run a proxy, the binary supports HTTP Basic Auth directly and is compatible with the BirdNET-Pi `CADDY_PWD` convention. This protects the UI with a username/password but is **still plain HTTP** — only use it behind TLS, or on a trusted LAN.

## A safer alternative: a private tunnel

For remote access without opening any ports, a mesh VPN like **Tailscale** or **WireGuard** is the easiest secure option: install it on the Pi and your phone/laptop, and reach `http://<tailscale-ip>:8502` as if you were on the same LAN — encrypted, no public exposure, no certificates to manage.

## Checklist

- [ ] Bind to `0.0.0.0` only if you need LAN/remote access.
- [ ] Never port-forward `8502` directly — always terminate TLS at a proxy or use a VPN.
- [ ] Add a password (proxy auth, built-in basic auth, or VPN-only access).
- [ ] Forward WebSocket upgrade headers in the proxy.
