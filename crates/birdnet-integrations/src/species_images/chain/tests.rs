//! Gates for the fallback chain.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// A provider that answers however the test says, and counts its calls.
///
/// The count is the point of half these gates: "did the fallback run" is not
/// answerable from the returned image alone when both providers could plausibly
/// have produced it.
struct Stub {
    answer: Result<&'static str, ImageError>,
    calls: Arc<AtomicUsize>,
}

impl Stub {
    fn found(url: &'static str) -> (Self, Arc<AtomicUsize>) {
        Self::new(Ok(url))
    }
    fn missing() -> (Self, Arc<AtomicUsize>) {
        Self::new(Err(ImageError::NotFound("Turdus merula".into())))
    }
    fn broken() -> (Self, Arc<AtomicUsize>) {
        Self::new(Err(ImageError::Api("Invalid API Key".into())))
    }
    fn new(answer: Result<&'static str, ImageError>) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                answer,
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

impl ImageProvider for Stub {
    fn fetch<'life0, 'life1, 'async_trait>(
        &'life0 self,
        _scientific_name: &'life1 str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<SpeciesImage, ImageError>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = match &self.answer {
            Ok(url) => Ok(SpeciesImage {
                url: (*url).to_string(),
                cached_path: None,
                width: 300,
                description: None,
                wiki_url: None,
            }),
            Err(ImageError::NotFound(s)) => Err(ImageError::NotFound(s.clone())),
            Err(ImageError::Api(s)) => Err(ImageError::Api(s.clone())),
            Err(e) => Err(ImageError::Api(e.to_string())),
        };
        Box::pin(async move { answer })
    }
}

/// A hit on the primary is used, and the fallback is never asked. Asking it
/// anyway would double every station's outbound requests for no benefit.
#[tokio::test]
async fn a_hit_on_the_primary_never_reaches_the_fallback() {
    let (first, first_calls) = Stub::found("https://flickr/photo.jpg");
    let (second, second_calls) = Stub::found("https://wikipedia/photo.jpg");
    let chain = FallbackProvider::new(Box::new(first), Box::new(second));

    let img = chain.fetch("Turdus merula").await.expect("the primary hit");
    assert_eq!(img.url, "https://flickr/photo.jpg");
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        second_calls.load(Ordering::SeqCst),
        0,
        "the fallback must not be consulted on a hit"
    );
}

/// A miss falls through. This is the whole reason the chain exists: an
/// operator who points the station at their own photostream has photographs of
/// a few dozen species, and the other nine thousand must still show something.
#[tokio::test]
async fn a_miss_on_the_primary_is_answered_by_the_fallback() {
    let (first, first_calls) = Stub::missing();
    let (second, second_calls) = Stub::found("https://wikipedia/photo.jpg");
    let chain = FallbackProvider::new(Box::new(first), Box::new(second));

    let img = chain
        .fetch("Turdus merula")
        .await
        .expect("the fallback hit");
    assert_eq!(img.url, "https://wikipedia/photo.jpg");
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 1);
}

/// A *broken* primary does not fall through. A wrong API key or a network
/// outage is something the operator can fix, and quietly serving the other
/// provider's image instead is how it stays broken for a year.
#[tokio::test]
async fn a_broken_primary_is_reported_rather_than_papered_over() {
    let (first, _) = Stub::broken();
    let (second, second_calls) = Stub::found("https://wikipedia/photo.jpg");
    let chain = FallbackProvider::new(Box::new(first), Box::new(second));

    let err = chain
        .fetch("Turdus merula")
        .await
        .expect_err("an API error must surface");
    assert!(
        err.to_string().contains("Invalid API Key"),
        "and carry the reason: {err}"
    );
    assert_eq!(
        second_calls.load(Ordering::SeqCst),
        0,
        "the fallback must not hide a fixable failure"
    );
}

/// Both empty is still a miss, reported as one, so the caller can render its
/// "no photograph" state rather than an error.
#[tokio::test]
async fn a_miss_on_both_stays_a_miss() {
    let (first, _) = Stub::missing();
    let (second, _) = Stub::missing();
    let chain = FallbackProvider::new(Box::new(first), Box::new(second));

    assert!(matches!(
        chain.fetch("Turdus merula").await,
        Err(ImageError::NotFound(_))
    ));
}
