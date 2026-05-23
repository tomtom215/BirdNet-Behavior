# Full Configuration Reference

This page mirrors the project's two canonical sources of truth **verbatim**:

- the documented [`.env.example`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/.env.example) template — every environment variable, and
- the binary's `--help` output — every CLI flag.

The `.env.example` block below is embedded straight from the repository at build time, so it can't drift from the code. For *how* settings layer together (CLI flags > environment variables > `birdnet.conf` > built-in defaults) and the web-UI-only settings, see [Configuration](../getting-started/configuration.md).

## Environment variables — `.env.example`

```dotenv
{{#include ../../../.env.example}}
```

## CLI flags — `birdnet-behavior --help`

```text
{{#include ../_generated/cli-help.txt}}
```

> **Keeping this page current:** the `.env.example` block updates itself on every docs build. Regenerate the CLI block after changing any flag with:
>
> ```bash
> ./scripts/gen-cli-help.sh
> ```
>
> CI fails if the committed `cli-help.txt` is stale, so this can't silently drift.
