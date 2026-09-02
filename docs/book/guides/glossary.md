# Glossary

Plain-English definitions for the terms you'll meet around the dashboard.

**Analysis window** — the three seconds of audio the model looks at in one go. A 15-second recording is five windows, or more if they [overlap](./tuning.md#10-asking-for-a-second-opinion).

**Banding code** — the four-letter shorthand ornithologists use for a species (e.g. **NOCA** = Northern Cardinal, **AMRO** = American Robin). BirdNet-Behavior shows it in each species' colored avatar.

**BirdNET / BirdNET+** — the neural network that identifies birds from sound, developed by the Cornell Lab of Ornithology. "BirdNET+ V3.0" is the specific model version this app uses.

**Co-occurrence (ρ)** — how often two species are detected together. The "ρ" (the Greek letter *rho*) is a correlation value from 0 to 1: higher means the two birds are heard together more than chance. See [Behavioral Analytics](../guide/analytics.md).

**Confidence** — the model's certainty in an identification, from 0 to 1 (shown as a percentage). A detection is only logged if its confidence clears the [threshold](./tuning.md#1-confidence-threshold).

**Dawn chorus** — the burst of bird song around sunrise. The [circadian polar plot](../guide/analytics.md) shows when each species is most vocal across the day.

**Detection** — one identification event: a species, a time, a confidence, and (usually) a short audio clip.

**Life list** — a birder's running list of every species they've ever recorded, each counted once. See [Species & the Life List](../guide/species.md).

**Mel spectrogram** — the visual "fingerprint" of a sound (frequency over time) that the model actually looks at. The live spectrogram shows it in real time.

**Offsite backup** — a copy of the database somewhere that is not the station's own SD card, encrypted before it leaves. The weekly snapshot in `backups/` protects against a corrupt page; only an offsite copy protects against the card. See [Backups & Recovery](../admin/backups.md#offsite-backups).

**Phenology** — the timing of seasonal events. **Migration phenology** is *when* migratory species arrive and depart through the year — the ridgeline chart on the [Migration](../guide/phenology.md) tab of Patterns.

**Quarantine** — a holding queue for borderline rare-bird detections, so a human can approve or reject them before they join your life list. See the [review queue](../guide/reviews.md#reviews-vs-quarantine).

**Repeat confirmation** — a filter that records a species only when enough of the analysis windows around it heard the same thing. A real bird sings across several; an artefact usually fires once. Needs overlapping windows to mean anything. See [Tuning](./tuning.md#10-asking-for-a-second-opinion).

**RTSP** — a streaming protocol many IP cameras use. BirdNet-Behavior can listen to an RTSP stream's audio as if it were a microphone.

**Sensitivity** — a BirdNET parameter (0.5–1.5) that makes the model more eager or more conservative. See [Tuning](./tuning.md#3-sensitivity).

**SF threshold (species-frequency)** — a filter that uses your location and the week of the year to down-weight species that shouldn't plausibly be present. See [Tuning](./tuning.md#4-species-frequency-sf-filter).

**SNR (signal-to-noise ratio)** — how loud the bird is relative to background noise, in decibels (dB). Higher is cleaner. Shown on the [Audio](../admin/audio.md) level meters.

**Streamgraph** — a flowing, stacked area chart (used for activity over time) drawn from a centered baseline so each band's *thickness* shows that species' share.

**Resident vs. migrant** — a resident species is present year-round; a migrant only passes through seasonally. Behavioral analytics classifies which is which from your data.
