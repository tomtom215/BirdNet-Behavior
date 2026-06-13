# Recordings & Live Audio

**Recordings** (`/recordings`) is the audio home — both the saved clips and the
live stream.

The home has two views, switched on the sub-tab row: **Clips** (browse what
your station already caught) and **Live** (listen along in real time).

## Clips (`/recordings?view=clips`)

![The recordings browser](../images/recordings.png)

Clips is a flat, newest-first list of every detection that saved an audio clip.
Each row carries the time, the species (with its banding-code avatar) and a
confidence bar; play it with the ▶ button, **download** it, **lock** it, or
delete it.

- **Filter chips** narrow the list to **All**, **Best** (high-confidence),
  **Rare** (a confident first-ever record) or **Locked** clips, and the search
  box finds a species by name. The chips and search are real links, so any
  view is bookmarkable.
- **Now playing** rides the page header while a clip plays and floats up into a
  compact bar when you scroll past it, with a live progress scrubber.
- **Lock** (🔒) protects a clip from the disk purge — the same lock you can set
  from [Today](./today.md); locked clips also show under the **Locked** filter.
- **Select** turns on checkboxes and a bulk bar so you can lock, download or
  delete a whole batch at once.

## Live (`/recordings?view=live`)

Open the **Live** view to listen along with your station in real time — a
per-source audio player and a scrolling spectrogram, with a live trickle of
detections as they're classified. The spectrogram is **honest**: it scrolls
only while audio is arriving and shows a flat baseline otherwise, never a fake
waveform. Today's live-signal card links straight here, and you can preselect a
microphone with `/recordings?view=live&source=<id>`.

> The pre-spine `/listen`, `/livestream` and `/live` addresses still work —
> they permanently redirect to `/recordings?view=live`.

## Species photos

The species **Gallery** now lives as the **Photos** view of the
[Species](./species.md) home (`/species?view=photos`) — a photo-card grid of
every detected species, searchable and sortable. The standalone `/gallery`
address still works.

![The species photo gallery](../images/gallery.png)

Each card shows the species photo (cached from Wikipedia, with attribution), the common and scientific name, the detection count, and a confidence pill. Species without a cached photo fall back to an on-brand placeholder tinted in the species' own color and stamped with its banding code — so the grid stays cohesive even before images load.

> All bird photos are cached locally from Wikimedia Commons under CC BY-SA, and the attribution is shown on each image as the license requires.
