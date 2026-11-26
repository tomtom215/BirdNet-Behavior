//! Shared fixtures for the daemon submodule unit tests.

use std::collections::HashMap;

/// Build a per-species threshold map from `(sci_name, threshold)` pairs.
pub(super) fn thresholds(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
}

/// What `f` logged on this thread at `level` and above, as text.
pub(super) fn logged<R>(level: tracing::Level, f: impl FnOnce() -> R) -> (R, String) {
    #[derive(Clone, Default)]
    struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buf = Buf::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(level)
        .finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    (out, text)
}
