# Migrating from BirdNET-Pi

BirdNet-Behavior imports an existing BirdNET-Pi history safely and **non-destructively** — the source database is opened read-only and never modified.

## Steps

1. Stop BirdNET-Pi so it isn't writing while you import:
   ```bash
   sudo systemctl stop birdnet_*
   ```
2. Open **`http://<your-pi>:8502/admin/migrate`**.
3. Upload or enter the path to your `~/BirdNET-Pi/BirdDB.txt`.
4. Review the preview — top 20 species, the date range, and a data-quality report.
5. Click **Import**. The import is transaction-backed and fails cleanly on any error, leaving your new database untouched if something goes wrong.
6. Verify the per-species count comparison.

Duplicate rows are silently skipped, so **re-running the import is safe** — if it's interrupted, just run it again.

## What carries over

The importer maps each legacy column to its new home and reports any it skips. Your detection history, timestamps, confidences and species names come across; BirdNET-Pi-specific config keys (`LATITUDE`, `SF_THRESH`, `SENSITIVITY`, …) are also understood when reading a `birdnet.conf`, so most settings feel familiar.

## Settings parity

BirdNet-Behavior reads a BirdNET-Pi-style `birdnet.conf`, so many of your existing keys (`ALSA_CARD`, `LATITUDE`/`LONGITUDE`, `OVERLAP`, `SF_THRESH`, `BIRDWEATHER_TOKEN`, …) are honored directly. See the mapping table in [Configuration](../getting-started/configuration.md).
