# Species Names in Your Language

BirdNet-Behavior can show bird names in 35 languages besides English. Choose one
under **Settings → System & Display → Common-name language**, or set
`DATABASE_LANG=de` in `birdnet.conf` (`--lang de`, or `BIRDNET_LANG`, work too).
It takes effect when the station next starts. A `--lang` given on the command
line wins over the Settings page and `birdnet.conf`, as every flag does, so the
page will show the stored choice while the station uses the flag's.

The name of a language is its code: `af`, `ar`, `ca`, `cs`, `da`, `de`, `en`, `es`,
`et`, `fi`, `fr`, `hr`, `hu`, `id`, `is`, `it`, `ja`, `ko`, `lt`, `lv`, `nl`, `no`,
`pl`, `pt`, `ro`, `ru`, `sk`, `sl`, `sr`, `sv`, `th`, `tr`, `uk`, `vi`, `zh_CN` and
`zh_TW`. A regional spelling (`de_AT`, `pt-BR`) and any capitalisation work. Chinese
has no plain `zh`: its two scripts are two sets of names, and the station will not
guess which you meant.

## Only what is shown changes

A detection is stored under the name the classifier gave it, in English, and
that stays true whatever language you choose. The language decides what is
*drawn*. That is different from BirdNET-Pi, which stores the translated name,
and it is deliberate:

- **Changing language loses nothing and splits nothing.** Last spring's
  *Eurasian Blackbird* and this spring's *Amsel* are the same bird in every list,
  count and chart, and you can switch back tomorrow.
- **Your exclusions keep working.** A species list that says "Dog" still matches
  the dog after you switch to German, because the list is matched against the
  classifier's names.
- **Links, forms, the JSON API, the exports, MQTT and webhooks carry the stored
  name.** A program that matches on `Eurasian Blackbird` keeps working; a link
  that said `name=Amsel` would open a page that has never heard of the bird.

Everything a person reads is translated: the Today feed and its top-species card,
the species list, photos and life list, a species' page, a detection's page, the
recordings and search results, the history and reports, the review queue and
quarantine, the dashboard, the kiosk, the share page, the Patterns charts, the RSS
and calendar feeds, notifications and the weekly report, and the live feed.

## How much of your classifier it covers

The names come from BirdNET-Pi's translation tables, which cover 7,058 species.
The BirdNET+ V3.0 classifier reports 11,560, and a language does not have a name
for every one of them: Swedish names nearly all the species in the tables,
Korean fewer than one in ten. A species with no name in your language keeps the
classifier's own, so a station set to Korean shows mostly English.

The language list on the Settings page shows, for your classifier, the share of
its species each language names, and `birdnet-behavior --doctor` prints the same
count for the language you have chosen. `GET /api/v2/languages` returns both.

These names are upstream's, not reviewed here; a wrong one is upstream's error.
If you want a name different from the one shown, or a language that is not in the
list, put a file named `de_labels.txt` (or `fr_labels.txt`, and so on) in a
directory and point `--labels-dir` (`LABELS_DIR`) at it. Each line is
`Scientific name_Common name`, and its names replace the built-in ones for that
language.

## Searching and typing names

You can type a name in your language into the species search, the command
palette (⌘K), the companion-species lookup, and the Today, Recordings and Search
boxes. You can also add one to an exclusion or allow list: it is saved as the
scientific name, which means the same thing in every language and survives a
change of language.

Three things still go by the classifier's name, because it is the stored name:

- **Sort order.** "Species A–Z" sorts the classifier's names, so a German list
  will not be alphabetical in German.
- **An alert rule's webhook body.** `{{species}}` is the stored name, and
  `{{sci_name}}` is the scientific name in every language. (A rule's *pattern*
  matches either: `Elster*` and `Eurasian*` both select the magpie on a German
  station. A notification's `$comname` is drawn in your language; its links carry
  the stored name.)
- **Entries typed before this existed.** A name typed into a species list
  used to be saved as typed. A German name saved that way matches nothing,
  because the classifier never reports it; remove it and add it again, and the
  scientific name is saved instead.

## Fonts

The interface's own typefaces cover Latin and Latin Extended. A name in another
script (Japanese, Korean, Chinese, Thai, Arabic, Cyrillic) is drawn by the
browser's own fonts, which every phone and desktop has. A bare Raspberry Pi
kiosk may not: if the wall display shows empty boxes where a name should be,
install a font that covers the script (`fonts-noto-cjk` for Chinese, Japanese and
Korean).

## Where the names come from

[`crates/birdnet-core/data/l18n/README.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/crates/birdnet-core/data/l18n/README.md)
says whose work they are, under what licence, how many names each language has,
and how to rebuild the table. The short version: BirdNET-Pi's `model/l18n/`
files at a pinned commit, reduced to the names that are genuine translations,
credited to @patlevin and Wikipedia by BirdNET-Pi itself.
