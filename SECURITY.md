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

Please **do not** put the details of a suspected vulnerability in a public
GitHub issue, pull request, or discussion.

**GitHub's private vulnerability reporting is switched off for this
repository.** On 2026-10-20 the repository setting answered
`{"enabled":false}` (`gh api repos/tomtom215/BirdNet-Behavior/private-vulnerability-reporting`),
so GitHub does not offer reporters the *Security → Report a vulnerability*
button. The `security/advisories/new` address that an earlier version of this
file gave is the maintainers' form for drafting an advisory, not a way for a
reporter to send one.

Until the maintainer turns reporting on, do this:

**Open an issue that contains no technical detail.** Title it "Security report —
please contact me", say that you have one, and say how the maintainer can reach
you privately (a GitHub profile with an email address on it, for instance). Do not
name the component, the version or the impact in the issue. The maintainer replies
by that route, and the details go there. No email address for security reports is
published in this repository.

That is slower and less private than an advisory, and it is the only route that
works today. This file will name GitHub's private reporting as the preferred
channel again once the setting is on.

When the exchange is private, include, where possible:

- A clear description of the issue and its impact (confidentiality,
  integrity, or availability)
- A minimal proof-of-concept (config snippet, request, audio clip, etc.)
- The version, deployment mode (Docker / bare metal / Pi image), and
  architecture (`x86_64`, `aarch64`)
- Any logs or screenshots that demonstrate the problem

Once a report has reached the maintainer, we aim to acknowledge it within
**72 hours** and to provide a remediation plan within **14 days**. We will credit reporters in the
release notes unless they request otherwise.

## Scope

In scope:

- Remote code execution, privilege escalation, or sandbox escape in any
  binary or container image we publish
- Authentication or authorisation bypass on the web UI, REST API, or
  admin panel (the session-cookie login, `BNB_API_TOKEN` bearer tokens,
  and signed share links)
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
