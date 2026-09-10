# Notifications & Integrations

BirdNet-Behavior can tell the rest of your world when a bird shows up — from a Telegram ping to a Home Assistant entity.

## Notification center

The **Notifications** page (`/notifications`) shows your channels and a log of recent events, with per-channel send counts, delivery status, and the last-sent time.

![The notification center](../images/notifications.png)

## Channels

- **Direct push** — paste one or more notification URLs under Settings →
  Notifications ("Notification URLs"), or set `BIRDNET_NOTIFY_URLS`. The syntax
  is Apprise's, so anything you already have written down works, but the station
  sends it itself: no Python, no `apprise` binary, no subprocess per detection.
  Handled natively: `discord://`, `slack://`, `tgram://`, `ntfy://`/`ntfys://`,
  `gotify://`/`gotifys://`, `pover://`, and `json://`/`jsons://` for a plain
  webhook.
- **Apprise** — for the other ~70 services. Point `BIRDNET_APPRISE_URL` at an
  [Apprise API server](https://github.com/caronc/apprise-api), or
  `BIRDNET_APPRISE_CONFIG` at an `apprise` config file. If every URL in that
  file is one of the schemes above, the `apprise` CLI is never invoked and you
  do not need it installed.
- **Email** — direct SMTP/STARTTLS with a per-species cooldown, configured under Settings → Notifications.
- **BirdWeather** — upload detections to your BirdWeather station. Set the token under Settings → Notifications, or with `BIRDNET_BIRDWEATHER_TOKEN`.

Either surface works for all three: the environment variable wins if you set it,
otherwise the value you save under Settings → Notifications is the one that
sends. The same applies to the minimum notification confidence
(`BIRDNET_NOTIFY_CONFIDENCE`, default `0.8`), the trigger mode, the cooldown,
the species allow/exclude lists, and the message templates. Changes take effect
on the next restart.

The **Send Test Push Notification** button under **Station → Alerts** (`/station/alerts`) uses the values saved on the
Settings page, so a successful test means live detections will notify too.

### Keeping the credential out of the environment

A notification URL carries its bot token *inside* the URL, and an environment
variable is readable by `docker inspect`, by anything with the process's
`/proc/<pid>/environ`, and by anything that logs its own environment. Docker and
Kubernetes both solve this by mounting the secret as a file; this station accepts
that convention:

| Instead of | Set | To the path of a file holding the value |
|---|---|---|
| `BIRDNET_NOTIFY_URLS` | `BIRDNET_NOTIFY_URLS_FILE` | e.g. `/run/secrets/birdnet_notify_urls` |
| `BIRDNET_APPRISE_URL` | `BIRDNET_APPRISE_URL_FILE` | |
| `BIRDNET_BIRDWEATHER_TOKEN` | `BIRDNET_BIRDWEATHER_TOKEN_FILE` | |
| `BIRDNET_MQTT_PASSWORD` | `BIRDNET_MQTT_PASSWORD_FILE` | |
| `BIRDNET_HEARTBEAT_URL` | `BIRDNET_HEARTBEAT_URL_FILE` | |

```yaml
services:
  birdnet:
    environment:
      BIRDNET_NOTIFY_URLS_FILE: /run/secrets/birdnet_notify_urls
    secrets:
      - birdnet_notify_urls
secrets:
  birdnet_notify_urls:
    file: ./secrets/notify_urls.txt
```

Four things worth knowing, each reported in the journal at startup:

- **The direct value wins.** Set both and the file is not read; the station says
  so at warning level rather than silently choosing one.
- **A file that cannot be read, or that is empty, leaves the feature off** — and
  says so at *error* level. A station that sends no notifications because a
  mount path had a typo looks exactly like one that was told to be quiet, so
  this is the one case worth an error line.
- **Surrounding whitespace is trimmed**, so the trailing newline every secret
  file has is harmless; inner newlines are kept, so a `NOTIFY_URLS` file may
  list one URL per line.
- **A value read from a file is not copied into the settings table**, so it
  stays out of the database and out of every backup, restore bundle and support
  archive taken from it. It will not appear on the Settings page either — which
  is the point.

`BIRDNET_APPRISE_CONFIG` has no `_FILE` form, because `APPRISE_CONFIG_FILE`
already exists and means something different: the path of an `apprise`
configuration file, which `apprise` itself reads. The SMTP password has none
either — it lives only in the settings table, set from the admin UI.

## MQTT & Home Assistant

A pure-Rust MQTT 3.1.1 client publishes detections to any broker (Mosquitto, Node-RED, EMQX, …) — no external broker library required.

```text
BIRDNET_MQTT_HOST=192.168.1.10
BIRDNET_MQTT_HA_DISCOVERY=1     # publish Home Assistant auto-discovery config
```

A broker the station cannot reach is reported, not just retried: after ten
minutes without a session the station-health alerts carry an `mqtt`
condition with the last error, `/api/v2/health` says `"mqtt": "disconnected"`,
and Home Assistant shows the station offline in the meantime. **Test all
channels** on the notifications test page publishes a real message to
`<prefix>/test` (never retained) and sends a test email through the configured
notifier, beside push and BirdWeather; a run in which no channel is configured
says that nothing was tested rather than that everything passed.

### TLS

```text
BIRDNET_MQTT_TLS=1              # port defaults to 8883 when this is on
BIRDNET_MQTT_CA_FILE=/etc/birdnet/mqtt-ca.pem
BIRDNET_MQTT_TLS_SERVER_NAME=broker.lan   # when connecting by IP
```

The certificate is always verified, against the system trust store plus
anything in `BIRDNET_MQTT_CA_FILE`. There is no option to skip verification:
that is the setting that gets switched on during setup and never switched off,
and an unverified TLS connection carrying the broker password is worse than a
plaintext one because it looks safe. Setting `BIRDNET_MQTT_CA_FILE` turns TLS
on by itself — configuring a trust anchor and then connecting in plaintext is
never what was meant.

Which certificate goes in that file depends on your broker:

- **Behind a private CA** — the *CA's* certificate. Pointing it at the broker's
  own certificate fails with `UnknownIssuer`.
- **Self-signed, no CA** — the broker's own certificate.

Either way the broker's certificate must carry `CA:FALSE`. A plain
`openssl req -x509` — what most "make a self-signed certificate" recipes give —
defaults to `CA:TRUE`, and the connection then fails with `CaUsedAsEndEntity`.
Add `-addext basicConstraints=critical,CA:FALSE` when generating it.

With `--mqtt-ha-discovery`, the station registers itself in Home Assistant automatically, so the latest detection, species count and confidence appear as entities you can put on a dashboard or trigger automations from.

## Alert rules

The **Rules** engine (**Station → Alerts**, `/station/alerts#rules`; the old `/admin/rules` redirects there) fires conditional actions on detections — for example, a webhook only when an owl is heard at night above 0.7 confidence, or a rule that suppresses a noisy false-positive species. Each rule matches on species pattern, confidence range, hour-of-day and day-of-week.

A webhook rule can authenticate, so it can target endpoints that need a key
rather than only ones that authenticate by URL alone:

| Scheme | Sends | Credential field |
|---|---|---|
| Bearer token | `Authorization: Bearer <token>` | the token |
| Basic | `Authorization: Basic base64(user:password)` | `user:password` |
| Custom header | `<name>: <value>` | the value, plus a header name |

Leaving the scheme on **None** keeps the request exactly as it was — which is
what every rule created before this existed does. The credential is stored in
the station's own database, is never rendered back into the page, and is
redacted from logs and from an exported rule set.

**Test** on a rule fires its action once, immediately, with an unmistakably
synthetic detection (`Test Detection (not a real bird)`) and reports the HTTP
status. Finding out an endpoint is wrong is worth doing when the rule is
written rather than the first time an owl calls at 3 a.m.

**Export** downloads the whole rule set as JSON with every credential replaced
by `***REDACTED***`, so it is safe to paste into a forum thread when asking for
help. **Export with credentials** is the backup-and-restore form and that file
is a secret. **Import** adds the rules in a pasted set — it never replaces what
is already there — and names any rule whose credential arrived redacted, since
those will fire unauthenticated until one is entered. One unusable entry is
reported and skipped rather than discarding the rest of the paste.
