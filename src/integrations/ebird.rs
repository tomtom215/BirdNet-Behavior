//! eBird recent-observations poll (`G-27`).
//!
//! Asks eBird once every [`REFRESH_INTERVAL`] what other people have reported
//! near the station, caches the answer to disk, and publishes it into
//! [`AppState`] where the suspect-species report and the detection detail page
//! consult it.
//!
//! # Off unless a key is configured
//!
//! There is no enable flag. eBird requires an API key for every endpoint, so a
//! station without `EBIRD_API_KEY` cannot reach the service and never tries —
//! [`birdnet_integrations::ebird::resolve`] returns `Ok(None)` and nothing is
//! spawned. That is the default state of a fresh install, and it phones nobody.
//!
//! The key is read the way every other credential is, so
//! `BIRDNET_EBIRD_API_KEY_FILE` mounts it from a file — see
//! `birdnet_core::config::secret_file`. Nothing here logs it.
//!
//! # The cache is read before the first fetch
//!
//! A station restarting at 03:00 should not have to wait for a round trip
//! before its pages can say anything. The cached snapshot is loaded and
//! published first, and only then does the loop fetch. A fetch that fails
//! leaves the published snapshot exactly as it was, so an offline station
//! keeps the last answer it got rather than losing the feature — the pages
//! label an old answer as old rather than hiding it.

use birdnet_core::config::Config;
use birdnet_integrations::ebird;
use birdnet_web::state::AppState;

/// Spawn the eBird poll when an API key is configured and there is somewhere
/// to ask about.
///
/// Returns the spawned task's handle when active, and `None` when no API key
/// is set (the default) or when the configuration cannot be used.
pub fn spawn_ebird_poll(
    config: Option<&Config>,
    state: AppState,
) -> Option<tokio::task::JoinHandle<()>> {
    // Each key is read as a literal `config.get("KEY")` rather than through a
    // helper closure: `tests/every_config_key_is_known.rs` finds reads by their
    // shape, and a closure hides the name from it.
    let settings = match ebird::resolve(
        config.and_then(|c| c.get("EBIRD_API_KEY")),
        config.and_then(|c| c.get("EBIRD_REGION")),
        config.and_then(|c| c.get("EBIRD_DIST_KM")),
        config.and_then(|c| c.get("EBIRD_BACK_DAYS")),
        resolve_lat(config),
        resolve_lon(config),
    ) {
        Ok(Some(settings)) => settings,
        Ok(None) => {
            tracing::debug!("eBird corroboration off (no EBIRD_API_KEY)");
            return None;
        }
        Err(e) => {
            // Named rather than swallowed: the operator supplied a key, so
            // they asked for this, and a silently dead integration looks
            // exactly like one that is working and finding nothing.
            tracing::warn!(error = %e, "eBird is configured but not usable; corroboration off");
            return None;
        }
    };

    let base = std::env::var("BNB_EBIRD_BASE_URL")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    let client = match ebird::Client::new(settings.api_key.clone(), base) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "eBird client init failed; corroboration off");
            return None;
        }
    };

    // Next to the database, which is the station's own data directory.
    let cache = ebird::cache_path(
        state
            .db_path()
            .parent()
            .unwrap_or_else(|| std::path::Path::new(".")),
    );

    tracing::info!(
        scope = %settings.scope.label(),
        back_days = settings.back_days,
        interval_secs = ebird::REFRESH_INTERVAL.as_secs(),
        "eBird corroboration enabled"
    );

    Some(tokio::spawn(async move {
        poll_loop(client, settings, cache, state, ebird::REFRESH_INTERVAL).await;
    }))
}

/// Publish the cached snapshot, then refresh it on `interval` forever.
async fn poll_loop(
    client: ebird::Client,
    settings: ebird::Settings,
    cache: std::path::PathBuf,
    state: AppState,
    interval: std::time::Duration,
) {
    // A snapshot taken for a different scope answers the wrong question, so
    // it is discarded rather than shown: an operator who moved the station or
    // changed the region would otherwise see corroboration from where it used
    // to be.
    let cached = {
        let path = cache.clone();
        tokio::task::spawn_blocking(move || ebird::load(&path))
            .await
            .ok()
            .flatten()
    };
    match cached {
        Some(snapshot) if snapshot.covers(&settings.scope) => {
            tracing::info!(
                species = snapshot.len(),
                age_secs = snapshot.age_secs(ebird::now_unix()),
                "eBird: serving the cached snapshot until the first fetch"
            );
            state.set_nearby(snapshot);
        }
        Some(_) => {
            tracing::info!("eBird: the cached snapshot is for a different area; ignoring it");
        }
        None => {}
    }

    loop {
        match client
            .recent(&settings.scope, settings.back_days, ebird::now_unix())
            .await
        {
            Ok(snapshot) => {
                let species = snapshot.len();
                let path = cache.clone();
                let to_store = snapshot.clone();
                let stored = tokio::task::spawn_blocking(move || ebird::store(&path, &to_store))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()));
                if let Err(e) = stored {
                    // Not fatal: the snapshot is still published, it just will
                    // not survive a restart.
                    tracing::warn!(error = %e, "eBird snapshot could not be cached");
                }
                state.set_nearby(snapshot);
                tracing::debug!(species, "eBird snapshot refreshed");
            }
            // A refused key never fixes itself, so it is an error rather than
            // a warning and it names the setting to change. Everything else is
            // worth retrying, and the published snapshot stays published —
            // this is the offline case the disk cache exists for.
            Err(e @ ebird::EbirdError::Unauthorized(_)) => {
                tracing::error!(
                    error = %e,
                    "eBird will keep refusing this key until it is corrected; \
                     corroboration will stay empty"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "eBird fetch failed; keeping the last snapshot");
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// The station's latitude, from the environment or the config file.
///
/// The same resolution order the weather poll uses, for the same reason: an
/// operator who set the coordinates once should not have to set them again.
fn resolve_lat(config: Option<&Config>) -> Option<f64> {
    resolve_decimal_env("BNB_STATION_LAT")
        .or_else(|| config.and_then(|cfg| cfg.get_parsed::<f64>("LATITUDE").ok()))
}

/// The station's longitude, from the environment or the config file.
fn resolve_lon(config: Option<&Config>) -> Option<f64> {
    resolve_decimal_env("BNB_STATION_LON")
        .or_else(|| config.and_then(|cfg| cfg.get_parsed::<f64>("LONGITUDE").ok()))
}

/// Read a decimal from the environment, tolerating a `,` decimal separator.
fn resolve_decimal_env(key: &str) -> Option<f64> {
    std::env::var(key)
        .ok()
        .and_then(|raw| birdnet_core::config::locale::parse_decimal(&raw).ok())
}
