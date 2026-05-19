# Getting help with BirdNet-Behavior

Pick the channel that matches what you need. Doing the right thing here
saves your time and the maintainer's.

## 1. Run the built-in diagnostic first

Almost every "it doesn't work" report turns out to be one of the items
the diagnostic already covers (missing audio source, wrong device name,
bad config value, disk almost full, model not yet downloaded). Run it
before anything else:

```bash
# Bare metal
sudo -u birdnet birdnet-behavior --doctor

# Docker
docker compose exec birdnet birdnet-behavior --doctor
```

The report prints a status, a message, and a concrete remediation for
every finding. Exit code 0 = ready; 1 = warnings only; 2 = at least one
error. If the diagnostic says everything is green and the system still
misbehaves, that is a useful data point — include the report in your
issue.

## 2. Search before you ask

- [Open and closed issues](https://github.com/tomtom215/BirdNet-Behavior/issues?q=is%3Aissue)
- [Discussions](https://github.com/tomtom215/BirdNet-Behavior/discussions)
- The `Troubleshooting` section of [`README.md`](README.md#troubleshooting)
- The architecture documents under [`docs/architecture/`](docs/architecture/),
  in particular [`12-risks.md`](docs/architecture/12-risks.md) and
  [`14-diagnostics.md`](docs/architecture/14-diagnostics.md)

## 3. Choose the right channel

| You want to                              | Use                                                                                              |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Report a bug                             | [Open an issue](https://github.com/tomtom215/BirdNet-Behavior/issues/new/choose) — bug form     |
| Suggest a feature                        | [Open an issue](https://github.com/tomtom215/BirdNet-Behavior/issues/new/choose) — feature form |
| Ask a how-to question                    | [Discussions](https://github.com/tomtom215/BirdNet-Behavior/discussions)                         |
| Share a build / station / mod            | [Discussions](https://github.com/tomtom215/BirdNet-Behavior/discussions)                         |
| Report a security vulnerability          | [Private security advisory](https://github.com/tomtom215/BirdNet-Behavior/security/advisories/new) — see [`SECURITY.md`](SECURITY.md) |

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

## 5. Response expectations

This is a volunteer-maintained project. Bug reports and PRs are reviewed
on a best-effort basis. Security reports get acknowledged within
72 hours per [`SECURITY.md`](SECURITY.md). Please be patient and kind.

## 6. Commercial support

There is no commercial support offering at present. If you need it, open
a discussion to gauge interest.
