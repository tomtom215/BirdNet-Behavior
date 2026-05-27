-- crates/birdnet-db/migrations/008_audio_sources.sql
-- O-13 · Audio Sources management — first-class CRUD storage.
--
-- The web layer currently keeps a single string in `settings.audio_source`.
-- This migration introduces a real table so the station can listen on
-- multiple inputs simultaneously, plus a soft-delete column so tombstones
-- are recoverable for audit / undo.
--
-- Roll back with `009_audio_sources_down.sql` (drops the table). The
-- `settings.audio_source` row is left intact across this migration so
-- audio-daemon code that still reads it continues to work.

CREATE TABLE audio_sources (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL
                  CHECK (kind IN ('usb-alsa','pipewire','rtsp')),
    device_id     TEXT NOT NULL,
    label         TEXT,
    sample_rate   INTEGER NOT NULL DEFAULT 48000
                  CHECK (sample_rate IN (8000, 16000, 22050, 32000, 44100, 48000)),
    channels      TEXT    NOT NULL DEFAULT 'mono'
                  CHECK (channels IN ('mono','left','right','stereo')),
    bit_depth     INTEGER NOT NULL DEFAULT 24
                  CHECK (bit_depth IN (16, 24)),
    gain_db       REAL    NOT NULL DEFAULT 0.0
                  CHECK (gain_db BETWEEN -24.0 AND 36.0),
    rtsp_transport TEXT   NOT NULL DEFAULT 'auto'
                  CHECK (rtsp_transport IN ('auto','tcp','udp')),
    schedule_quiet_start  TEXT,        -- HH:MM 24h, NULL = no schedule
    schedule_quiet_end    TEXT,
    pipeline_high_pass        INTEGER NOT NULL DEFAULT 1,
    pipeline_dc_removal       INTEGER NOT NULL DEFAULT 1,
    pipeline_agc              INTEGER NOT NULL DEFAULT 0,
    pipeline_rtsp_keepalive   INTEGER NOT NULL DEFAULT 1,
    disabled_at   TEXT,                -- ISO-8601 when soft-deleted
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX audio_sources_kind_active
    ON audio_sources (kind, disabled_at);

-- Seed row #1 from the existing single-string config so anyone with a
-- configured `audio_source` setting lands on a populated page.
INSERT INTO audio_sources (id, kind, device_id, label)
SELECT 'src_seed_1',
       CASE
         WHEN value LIKE 'rtsp://%' THEN 'rtsp'
         WHEN value LIKE 'alsa_%'   THEN 'pipewire'
         ELSE 'usb-alsa'
       END,
       value,
       NULL
  FROM settings
 WHERE key = 'audio_source'
   AND value IS NOT NULL
   AND value <> ''
   AND NOT EXISTS (SELECT 1 FROM audio_sources LIMIT 1);

-- Keep settings.audio_source pointing at the seed row's device_id so the
-- audio daemon (which still reads the single string) doesn't break in this
-- migration. Once the daemon reads from audio_sources directly, the
-- settings row can be retired.
