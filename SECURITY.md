# Security Policy

BirdNet-Behavior runs unattended on home networks and listens to a microphone
in someone's home, garden, or research site. We take its security posture
seriously and treat it as a piece of safety-relevant infrastructure even
though it is not life-critical.

## Supported versions

We provide security fixes for the **latest released minor version** on the
default branch. Older releases are best-effort.

| Version           | Security fixes | Notes                              |
| ----------------- | -------------- | ---------------------------------- |
| `main` / `edge`   | Yes            | Rolling fixes land here first      |
| Latest `v0.x.y`   | Yes            | Backported when feasible           |
| Older `v0.x.*`    | No             | Upgrade required                   |

We do not yet maintain a long-term-support branch. When the project reaches
a `v1.0` release this policy will be revisited and a clear support window
will be added here.

## Reporting a vulnerability

Please **do not** open a public GitHub issue, pull request, or discussion
for suspected vulnerabilities. Instead, use one of the following private
channels so we can coordinate disclosure:

1. **GitHub Security Advisories (preferred).** Open a private advisory at
   <https://github.com/tomtom215/BirdNet-Behavior/security/advisories/new>.
2. **Email.** Send an encrypted message to the maintainer listed on the
   repository profile, with `[birdnet-behavior security]` in the subject.

Please include, where possible:

- A clear description of the issue and its impact (confidentiality,
  integrity, or availability)
- A minimal proof-of-concept (config snippet, request, audio clip, etc.)
- The version, deployment mode (Docker / bare metal / Pi image), and
  architecture (`x86_64`, `aarch64`)
- Any logs or screenshots that demonstrate the problem

We aim to acknowledge new reports within **72 hours** and to provide a
remediation plan within **14 days**. We will credit reporters in the
release notes unless they request otherwise.

## Scope

In scope:

- Remote code execution, privilege escalation, or sandbox escape in any
  binary or container image we publish
- Authentication or authorisation bypass on the web UI, REST API, or
  admin panel (including the optional HTTP Basic Auth layer)
- Path traversal, SSRF, or arbitrary file read/write in audio handling,
  recording downloads, or model loading
- Cryptographic weakness in stored credentials, tokens, or backup
  artefacts
- Supply-chain compromise of our release pipeline, container images, or
  published binaries
- Logic flaws in the recording-retention or quarantine system that could
  delete data the operator intended to keep, or surface data they intended
  to suppress

Out of scope:

- Issues that require the attacker to already be `root` on the host (or
  the equivalent Docker socket access)
- Vulnerabilities in third-party services that the operator has chosen to
  integrate (BirdWeather, Apprise endpoints, MQTT brokers) when used as
  documented
- Findings from automated scanners with no demonstrated exploit path
- Best-practice suggestions without a concrete impact — these are welcome
  as normal GitHub issues

## Threat model

A high-level threat model and risk register live in
[`docs/architecture/12-risks.md`](docs/architecture/12-risks.md). It
covers the deployment topology, trust boundaries, and the mitigations
currently in place. Reporters are encouraged (but not required) to map
findings to that document.

## Hardening guidance

If you are deploying BirdNet-Behavior in a multi-tenant or research
setting, please also review [`docs/book/field/hardening.md`](docs/book/field/hardening.md)
for recommended network segmentation, authentication, and backup
practices.
