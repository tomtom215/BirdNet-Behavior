# Getting help with BirdNet-Behavior

Pick the channel that matches what you need. Doing the right thing here
saves your time and the maintainer's.

## 1. Run the built-in diagnostic first

Almost every "it doesn't work" report turns out to be one of the items
the diagnostic already covers (missing audio source, wrong device name,
bad config value, disk almost full, model not yet downloaded). Run it
before anything else:

```bash
# Bare metal: the service runs as the account that ran the installer (there is
# no "birdnet" user); this prints that account and runs the doctor as it
sudo -u "$(systemctl show -p User --value birdnet-behavior)" birdnet-behavior --doctor

# Docker
docker compose exec birdnet birdnet-behavior --doctor
```

The report prints a status, a message, and a concrete remediation for
every finding. Exit code 0 = ready; 1 = warnings only; 2 = at least one
error. With a browser but no shell, sign in and open `/admin/doctor`: it
renders the same report, serves it as JSON at `/admin/doctor.json`, and
offers a redacted support bundle to download. If the diagnostic says everything is green and the system still
misbehaves, that is a useful data point — include the report in your
issue.

## 2. Search before you ask

- [Open and closed issues](https://github.com/tomtom215/BirdNet-Behavior/issues?q=is%3Aissue)
- The [troubleshooting guide](docs/book/guides/troubleshooting.md), which
  [`TROUBLESHOOTING.md`](TROUBLESHOOTING.md) points to
- The `Troubleshooting` section of [`README.md`](README.md#troubleshooting)
- The architecture documents under [`docs/architecture/`](docs/architecture/),
  in particular [`12-risks.md`](docs/architecture/12-risks.md) and
  [`14-diagnostics.md`](docs/architecture/14-diagnostics.md)

## 3. Choose the right channel

| You want to                              | Use                                                                                              |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Report a bug                             | [Open an issue](https://github.com/tomtom215/BirdNet-Behavior/issues/new/choose) — bug form     |
| Suggest a feature                        | [Open an issue](https://github.com/tomtom215/BirdNet-Behavior/issues/new/choose) — feature form |
| Ask a how-to question                    | [Open an issue](https://github.com/tomtom215/BirdNet-Behavior/issues/new/choose) and say it is a question (see the note below) |
| Share a build / station / mod            | [Open an issue](https://github.com/tomtom215/BirdNet-Behavior/issues/new/choose) and say what it is |
| Report a security vulnerability          | Not through a public issue. GitHub's private reporting is **off** at the moment — [`SECURITY.md`](SECURITY.md) says what to do instead |

> GitHub Discussions are not enabled for this repository (the REST API answered
> `has_discussions: false` on 2026-10-20), so older versions of this file that
> pointed questions there pointed at a page that does not exist. If the maintainer
> turns them on, how-to questions belong there.

## 4. What to include in a bug report

The bug-report form already asks for these; humour the maintainer and
fill them in:

- Output of `birdnet-behavior --doctor` (or the Docker equivalent)
- Steps to reproduce, in order
- Deployment mode (Docker / bare metal / Pi image / source build)
- Architecture (`uname -m`) and OS version
- BirdNet-Behavior version (`birdnet-behavior --version` or the image
  digest from `docker image inspect`)
- Last ~50 lines of `journalctl -u birdnet-behavior` or
  `docker compose logs birdnet`

`birdnet-behavior --support-bundle` (also `/admin/support-bundle` behind the
admin login) collects the report, the version, a redacted configuration and the
last 2000 journal lines into one `.tar.gz`. Passwords, tokens and URL
credentials are masked, and **the station's `LATITUDE` and `LONGITUDE` are shown
to one decimal place (about 11 km)** in the configuration section and wherever
the journal printed them: enough to say which hemisphere, season and region the
station is in, not enough to find a house. Read the archive before posting it in
a public issue all the same.

## 5. Response expectations

This is a volunteer-maintained project. Bug reports and PRs are reviewed
on a best-effort basis. Security reports get acknowledged within
72 hours per [`SECURITY.md`](SECURITY.md). Please be patient and kind.

## 6. Commercial support

There is no commercial support offering at present. If you need it, open
a discussion to gauge interest.
