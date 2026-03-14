//! BirdNET-Pi → BirdNet-Behavior migration.
//!
//! Provides safe, tested, idempotent, rollback-friendly import of existing
//! BirdNET-Pi detection databases.
//!
//! # Overview
//!
//! Migration is a three-step process:
//!
//! 1. **Detect** — [`schema::detect_schema`] inspects the source file read-only
//!    and identifies its schema.
//! 2. **Validate** — [`traits::Validator`] runs pre-flight checks (row count,
//!    date format, confidence range).
//! 3. **Import** — [`traits::Migrator`] copies rows in batches using
//!    `INSERT OR IGNORE` so the operation is idempotent.
//!
//! The source file is **never modified**.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use birdnet_migrate::birdnet_pi;
//! use birdnet_migrate::progress::ProgressHandle;
//!
//! let progress = ProgressHandle::new();
//! let summary = birdnet_pi::run_migration(
//!     std::path::Path::new("/home/pi/BirdNET-Pi/scripts/BirdDB.txt"),
//!     std::path::Path::new("/home/pi/BirdNet-Behavior/birds.db"),
//!     false,  // strict = false → validation warnings don't abort
//!     &progress,
//! ).unwrap();
//!
//! println!("Imported {} rows", summary.imported_rows);
//! ```

pub mod birdnet_pi;
pub mod error;
pub mod progress;
pub mod schema;
pub mod traits;

pub use birdnet_pi::csv_importer::CsvImporter;
pub use birdnet_pi::species_report::{
    MigrationReport, PostMigrationReport, SpeciesDiff, SpeciesStats,
};
pub use error::MigrateError;
pub use progress::{MigrationProgress, MigrationStage, ProgressHandle};
pub use schema::DetectedSchema;
pub use traits::{MigrationSummary, ValidationReport};
