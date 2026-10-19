//! The CSV importer's log must not quote the file it reads.
//!
//! The source is whatever path an admin typed into the migration form, so a
//! line that does not parse can be the contents of any file the service user
//! can read. The journal outlives the request and goes into the support bundle.
//!
//! This is its own test binary, with this one test in it, on purpose. A
//! `tracing` call site caches whether any subscriber wants it, and a scoped
//! subscriber installed with `with_default` can lose a race against another
//! thread that reached the same call site first with none installed: in the
//! lib's test binary, where other tests import bad lines in parallel, this
//! assertion saw an empty log whenever it ran on more than one thread.

use std::io::Write as _;
use std::sync::{Arc, Mutex};

use birdnet_migrate::{CsvImporter, ProgressHandle, traits::Migrator};

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
    type Writer = Self;
    fn make_writer(&'a self) -> Self {
        self.clone()
    }
}

#[test]
fn an_unparseable_line_is_logged_by_number_and_not_by_its_text() {
    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(sink.clone())
        .with_ansi(false)
        .finish();

    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("BirdDB.txt");
    let mut f = std::fs::File::create(&src).unwrap();
    f.write_all(
        b"2026-01-15;06:23:11;Turdus merula;Eurasian Blackbird;0.921;51.5;-0.1;0.7;3;1.0;0.0;rec.wav\n\
          ALPHA_TOKEN=s3cr3t-token-123\n",
    )
    .unwrap();
    drop(f);
    let dst = dir.path().join("dest.db");

    let summary = tracing::subscriber::with_default(subscriber, || {
        CsvImporter.migrate(&src, &dst, &ProgressHandle::new())
    })
    .expect("one good row imports");
    assert_eq!(summary.imported_rows, 1);

    let log = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains("skipping unparseable CSV line") && log.contains("data_line=2"),
        "the skip must be logged, with the line number (precondition): {log}"
    );
    assert!(
        !log.contains("s3cr3t-token-123"),
        "the text of a skipped line reached the log: {log}"
    );
}
