# Troubleshooting BirdNet-Behavior

> "Why isn't it working?" — read this *before* opening an issue.

The troubleshooting guide lives in the manual, where it is versioned with each
release and ships inside the binary (offline, at `/help/guides/troubleshooting`
on your station). This file is only the way in, so that there is one set of
advice and not two.

- **[The guide, as it is in this tree](docs/book/guides/troubleshooting.md)** —
  by symptom: the service will not start, the web UI is unreachable, no
  detections, model not found, database errors, analytics problems, memory
  pressure, notifications.
- The same page as published for the latest release:
  <https://tomtom215.github.io/BirdNet-Behavior/guides/troubleshooting.html>
  (and for the development tree, under `/dev/`).

## Always start here: run the doctor

```bash
# Bare metal. The service runs as the account that ran the installer; there is
# no "birdnet" user, and this prints that account and runs the doctor as it.
sudo -u "$(systemctl show -p User --value birdnet-behavior)" birdnet-behavior --doctor

# Docker
docker compose exec birdnet birdnet-behavior --doctor
```

It prints a one-screen report with a concrete fix for every finding. Exit code
`0` is all good, `1` is warnings only, `2` is at least one error. With a browser
and no shell, sign in and open `/admin/doctor`.

## If nothing helps

Open a [bug report](https://github.com/tomtom215/BirdNet-Behavior/issues/new/choose)
with the doctor's output; [`SUPPORT.md`](SUPPORT.md) says what else to include,
and the bundle written by `birdnet-behavior --support-bundle` collects most of it
(it shows the station's coordinates to one decimal place only). Do not put a
suspected vulnerability in an issue: see [`SECURITY.md`](SECURITY.md).

Related: [`README.md` § Troubleshooting](README.md#troubleshooting),
[`docs/architecture/12-risks.md`](docs/architecture/12-risks.md) (the risk
register) and [`docs/architecture/14-diagnostics.md`](docs/architecture/14-diagnostics.md)
(why the doctor checks what it does).
