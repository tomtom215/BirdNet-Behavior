//! Public-domain plates: nineteenth-century bird illustrations, by scientific name.
//!
//! The kiosk draws today's birds as a page from a field guide, and a plate drawn by Gould or
//! Naumann belongs on that page more than a photograph does. The pictures are not bundled: this
//! crate carries only an *index*, a few hundred kilobytes naming, for each species, the one
//! Wikimedia Commons file to use, who made it and under what terms. A station fetches a plate
//! the first time the kiosk asks for it and keeps it, as it does a photograph, through the same
//! [`ImageCache`](super::ImageCache) (so the size cap, the miss memory and the credit kept beside
//! the file are the ones photographs have).
//!
//! # What is in the index, and what is not
//!
//! `tools/plate-index/build.py` builds it from Commons and keeps a file only when Commons' own
//! metadata calls it free of copyright (public domain, CC0, Public Domain Mark) and it shows one
//! species. A scan re-licensed by its uploader (CC BY-SA, say) is left out even though the
//! 19th-century work under it is old: a CC BY-SA plate cannot go into this CC BY-NC-SA project.
//! Coverage is therefore partial, and a species with no plate gets its photograph, engraved.
//!
//! # Credit
//!
//! A public-domain work needs no credit, and every plate is still credited: the work, its
//! author, the file it is on and its licence, as an [`ImageCredit`] like any other picture's.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use super::provider::ImageProvider;
use super::types::{ImageCredit, ImageError, SpeciesImage};

/// How wide, in pixels, a plate is asked for. A figure on a 4K wall display is about 800 pixels
/// across at its largest; a plate is a flat picture and compresses well at this size.
pub const PLATE_WIDTH: u32 = 900;

/// The licences a plate may carry, by the short name Commons gives them: free of copyright, and
/// nothing that asks for attribution or a share-alike. The index builder applies the same rule;
/// this is the second line of defence if an index is edited by hand.
const ACCEPTED_LICENCES: [&str; 3] = ["public domain", "cc0", "public domain mark"];

/// One source work: a book of plates.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Work {
    /// The book's title.
    pub title: String,
    /// Its author or authors.
    pub author: String,
    /// The years it was published over, `1832-1837`.
    pub years: String,
    /// The Commons category its plates were found in.
    #[serde(default)]
    pub commons_category: String,
}

/// One plate: the file chosen for a species.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Plate {
    /// The key of the [`Work`] it is from.
    pub work: String,
    /// The Commons file name, without `File:`.
    pub file: String,
    /// The licence's short name on Commons.
    pub licence: String,
    /// The licence's page, or empty.
    #[serde(default)]
    pub licence_url: String,
    /// The file's page on Commons.
    pub page_url: String,
}

/// The index: which species have a plate, and which book each is from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct PlateIndex {
    #[serde(default)]
    works: BTreeMap<String, Work>,
    #[serde(default)]
    plates: BTreeMap<String, Plate>,
}

impl PlateIndex {
    /// Parse an index from its JSON text, keeping only what may be used.
    ///
    /// A plate whose work is not described, whose licence is not a free one, or that names no
    /// page to link to, is dropped rather than served: the index is data a person built, and
    /// this is where a mistake in it is stopped.
    ///
    /// # Errors
    ///
    /// [`ImageError::Api`] when the text is not the expected JSON.
    pub fn parse(json: &str) -> Result<Self, ImageError> {
        let bad = |e: &dyn std::fmt::Display| {
            ImageError::Api(format!("the plate index is not the expected JSON: {e}"))
        };
        // An object, and nothing else: serde would read `[]` as a struct of defaults, an index
        // that is quietly empty.
        let value: serde_json::Value = serde_json::from_str(json).map_err(|e| bad(&e))?;
        if !value.is_object() {
            return Err(bad(&"the top level is not an object"));
        }
        let mut index: Self = serde_json::from_value(value).map_err(|e| bad(&e))?;
        let works = index.works.clone();
        index.plates.retain(|_, p| {
            works.contains_key(&p.work)
                && ACCEPTED_LICENCES.contains(&p.licence.trim().to_ascii_lowercase().as_str())
                && p.page_url.starts_with("https://")
                && !p.file.is_empty()
        });
        Ok(index)
    }

    /// The index built into this binary.
    ///
    /// # Panics
    ///
    /// Never in a build that passes its tests: `the_embedded_index_parses` fails first.
    #[must_use]
    pub fn embedded() -> &'static Self {
        static INDEX: OnceLock<PlateIndex> = OnceLock::new();
        INDEX.get_or_init(|| {
            Self::parse(include_str!("../../data/plates/index.json")).unwrap_or_default()
        })
    }

    /// Number of species with a plate.
    #[must_use]
    pub fn len(&self) -> usize {
        self.plates.len()
    }

    /// Whether the index holds no plates.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plates.is_empty()
    }

    /// The plate for a scientific name, case-insensitively, and the book it is from.
    #[must_use]
    pub fn plate_for(&self, scientific_name: &str) -> Option<(&Plate, &Work)> {
        let wanted = scientific_name.trim();
        let (_, plate) = self
            .plates
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))?;
        Some((plate, self.works.get(&plate.work)?))
    }

    /// The works that at least one plate is from, in key order.
    pub fn works_in_use(&self) -> impl Iterator<Item = (&String, &Work)> {
        self.works
            .iter()
            .filter(|(key, _)| self.plates.values().any(|p| &p.work == *key))
    }

    /// The credit a plate is shown with.
    #[must_use]
    pub fn credit_for(&self, scientific_name: &str) -> Option<ImageCredit> {
        let (plate, work) = self.plate_for(scientific_name)?;
        Some(ImageCredit {
            title: format!("{}, plate from {}", scientific_name.trim(), work.citation()),
            // The work's author, not the file's free-text one: on Commons that field is whatever the
            // uploader typed, and for the Naumann plates it carries the digitiser's name and an
            // email address. The file's page is linked for anyone who wants it.
            author: work.author.clone(),
            source: "Wikimedia Commons".to_owned(),
            source_url: plate.page_url.clone(),
            licence: plate.licence.clone(),
            licence_url: plate.licence_url.clone(),
        })
    }
}

impl Work {
    /// The work as a citation: `The Birds of Europe (1832–1837)`.
    #[must_use]
    pub fn citation(&self) -> String {
        if self.years.is_empty() {
            self.title.clone()
        } else {
            format!("{} ({})", self.title, self.years.replace('-', "–"))
        }
    }
}

/// An [`ImageProvider`] over a [`PlateIndex`]: it answers from the index and names the thumbnail
/// to download; the cache around it does the downloading.
#[derive(Debug, Clone)]
pub struct PlateProvider {
    index: PlateIndex,
    width: u32,
}

impl PlateProvider {
    /// A provider over `index`.
    #[must_use]
    pub const fn new(index: PlateIndex, width: u32) -> Self {
        Self { index, width }
    }

    /// A provider over the index built into this binary.
    #[must_use]
    pub fn embedded() -> Self {
        Self::new(PlateIndex::embedded().clone(), PLATE_WIDTH)
    }

    /// The Commons address a plate's thumbnail is fetched from.
    fn thumb_url(&self, file: &str) -> String {
        format!(
            "https://commons.wikimedia.org/wiki/Special:FilePath/{}?width={}",
            super::wikipedia::url_encode(&file.replace(' ', "_")),
            self.width
        )
    }
}

impl ImageProvider for PlateProvider {
    fn fetch<'life0, 'life1, 'async_trait>(
        &'life0 self,
        scientific_name: &'life1 str,
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
        let found = self.index.plate_for(scientific_name).map(|(plate, work)| {
            (
                self.thumb_url(&plate.file),
                plate.page_url.clone(),
                work.citation(),
            )
        });
        let credit = self.index.credit_for(scientific_name);
        let name = scientific_name.to_owned();
        Box::pin(async move {
            let (url, page_url, citation) = found.ok_or(ImageError::NotFound(name))?;
            Ok(SpeciesImage {
                url,
                cached_path: None,
                width: self.width,
                description: Some(citation),
                wiki_url: Some(page_url),
                credit,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> &'static str {
        r#"{
          "schema": 1,
          "works": {
            "gould-europe": {"title": "The Birds of Europe", "author": "John Gould",
                             "years": "1832-1837", "commons_category": "Category:The Birds of Europe (Gould)"},
            "naumann": {"title": "Naturgeschichte der Vögel Mitteleuropas", "author": "J. F. Naumann",
                        "years": "1897-1905"}
          },
          "plates": {
            "Turdus pilaris": {"work": "gould-europe", "file": "Turdus pilaris John Gould. The birds of Europe 2. 1837.jpg",
                               "author": "J and E Gould E.Lear", "licence": "Public domain",
                               "licence_url": "https://creativecommons.org/publicdomain/mark/1.0/",
                               "page_url": "https://commons.wikimedia.org/wiki/File:Turdus_pilaris.jpg"},
            "Pica pica": {"work": "naumann", "file": "Pica pica NAUMANN.jpg", "licence": "CC0",
                          "author": "NAUMANN, NATURGESCHICHTE DER VÖGEL MITTELEUROPAS: Band III - digitale Bearbeitung : P. S. - b-online@example.org",
                          "page_url": "https://commons.wikimedia.org/wiki/File:Pica_pica_NAUMANN.jpg"},
            "Bad licence": {"work": "naumann", "file": "x.jpg", "licence": "CC BY-SA 4.0",
                            "page_url": "https://commons.wikimedia.org/wiki/File:x.jpg"},
            "Unknown work": {"work": "nobody", "file": "y.jpg", "licence": "Public domain",
                             "page_url": "https://commons.wikimedia.org/wiki/File:y.jpg"},
            "No page": {"work": "naumann", "file": "z.jpg", "licence": "Public domain", "page_url": ""}
          }
        }"#
    }

    #[test]
    fn only_free_plates_with_a_known_work_and_a_page_survive_parsing() {
        let index = PlateIndex::parse(sample()).unwrap();
        assert_eq!(index.len(), 2, "{index:?}");
        assert!(index.plate_for("Turdus pilaris").is_some());
        assert!(index.plate_for("Pica pica").is_some());
        for refused in ["Bad licence", "Unknown work", "No page"] {
            assert!(index.plate_for(refused).is_none(), "{refused}");
        }
    }

    /// A share-alike plate is refused here even if the builder let it through: the index is data
    /// a person built, and a mistake in it must not become a licence breach on a wall.
    #[test]
    fn a_share_alike_licence_is_never_served() {
        let share_alike = sample().replace(
            "\"licence\": \"Public domain\"",
            "\"licence\": \"CC BY-SA 4.0\"",
        );
        let index = PlateIndex::parse(&share_alike).unwrap();
        assert!(index.plate_for("Turdus pilaris").is_none());
        assert!(index.plate_for("Pica pica").is_some(), "CC0 is still free");
    }

    #[test]
    fn a_plate_is_found_by_name_in_any_case() {
        let index = PlateIndex::parse(sample()).unwrap();
        assert!(index.plate_for("  turdus PILARIS ").is_some());
        assert!(index.plate_for("Turdus merula").is_none());
    }

    #[test]
    fn the_credit_names_the_work_the_author_the_file_and_the_licence() {
        let index = PlateIndex::parse(sample()).unwrap();
        let credit = index.credit_for("Turdus pilaris").unwrap();
        assert_eq!(
            credit.title,
            "Turdus pilaris, plate from The Birds of Europe (1832–1837)"
        );
        assert_eq!(credit.author, "John Gould");
        assert_eq!(credit.source, "Wikimedia Commons");
        assert_eq!(credit.licence, "Public domain");
        assert!(credit.is_attributable() && !credit.licence_is_restricted());
        // Credited to the work's author, never the file's free text, which on Commons is whatever
        // the uploader typed: for the real Naumann plates it carries an email address.
        for name in ["Turdus pilaris", "Pica pica"] {
            let credit = index.credit_for(name).unwrap();
            assert!(!credit.author.contains('@'), "{name}: {}", credit.author);
        }
        assert_eq!(
            index.credit_for("Pica pica").unwrap().author,
            "J. F. Naumann"
        );
        assert_eq!(
            index.credit_for("Turdus pilaris").unwrap().author,
            "John Gould"
        );
    }

    #[test]
    fn the_works_in_use_are_those_with_a_plate() {
        let mut json: serde_json::Value = serde_json::from_str(sample()).unwrap();
        json["works"]["unused"] = serde_json::json!({"title": "T", "author": "A", "years": ""});
        let index = PlateIndex::parse(&json.to_string()).unwrap();
        let keys: Vec<&str> = index.works_in_use().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["gould-europe", "naumann"]);
    }

    #[tokio::test]
    async fn the_provider_names_a_thumbnail_and_its_credit_and_refuses_the_rest() {
        let provider = PlateProvider::new(PlateIndex::parse(sample()).unwrap(), 900);
        let img = provider.fetch("Turdus pilaris").await.unwrap();
        assert_eq!(
            img.url,
            "https://commons.wikimedia.org/wiki/Special:FilePath/\
             Turdus_pilaris_John_Gould._The_birds_of_Europe_2._1837.jpg?width=900"
        );
        assert!(
            img.credit
                .as_ref()
                .is_some_and(ImageCredit::is_attributable)
        );
        assert_eq!(
            img.description.as_deref(),
            Some("The Birds of Europe (1832–1837)")
        );
        assert!(matches!(
            provider.fetch("Turdus merula").await,
            Err(ImageError::NotFound(_))
        ));
    }

    #[test]
    fn a_malformed_index_is_an_error_not_an_empty_one() {
        assert!(matches!(PlateIndex::parse("[]"), Err(ImageError::Api(_))));
        assert!(matches!(
            PlateIndex::parse("not json"),
            Err(ImageError::Api(_))
        ));
    }

    /// The index that ships. Read as the raw file as well as through `parse`, so a plate the parser
    /// drops (a licence spelled a way it does not accept, a work it does not describe) is a failure
    /// here and not a quiet loss of coverage.
    #[test]
    fn the_embedded_index_parses_and_nothing_in_it_is_dropped() {
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../../data/plates/index.json")).unwrap();
        let listed = raw["plates"].as_object().unwrap().len();
        let index = PlateIndex::embedded();
        assert!(listed >= 400, "the index lists only {listed} plates");
        assert_eq!(
            index.len(),
            listed,
            "the parser dropped plates the index lists"
        );
        assert!(index.works_in_use().count() >= 3);
        // A bird every plate collection has, with a credit that names the work and carries no email.
        let credit = index
            .credit_for("Turdus merula")
            .expect("a blackbird plate");
        assert!(
            credit.is_attributable() && !credit.licence_is_restricted(),
            "{credit:?}"
        );
        for name in index.plates.keys() {
            let credit = index.credit_for(name).unwrap();
            assert!(
                !credit.author.contains('@') && !credit.author.is_empty(),
                "{name}: {credit:?}"
            );
            assert!(
                credit
                    .source_url
                    .starts_with("https://commons.wikimedia.org/wiki/File:"),
                "{name}"
            );
        }
    }
}
