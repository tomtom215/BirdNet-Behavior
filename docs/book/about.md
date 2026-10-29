# Credits & License

Who built what this project stands on, and the terms it is released under; read this before reusing the software or its data, or if you are the author of one of the projects named.

## Lineage & attribution

BirdNet-Behavior stands on the shoulders of the conservation-bioacoustics community. It is a **rewrite of BirdNET-Pi in Rust, derived from it and licensed accordingly** (the repository's `LICENSE` says it "is derived from and inspired by" the upstream works below).

- **[BirdNET](https://github.com/kahst/BirdNET-Analyzer)** — the ML model, by the K. Lisa Yang Center for Conservation Bioacoustics, Cornell Lab of Ornithology.
- **[BirdNET-Pi](https://github.com/mcguirepr89/BirdNET-Pi)** — the original Raspberry Pi implementation, by [Patrick McGuire](https://github.com/mcguirepr89).
- **[BirdNET-Pi fork](https://github.com/Nachtzuster/BirdNET-Pi)** — the maintained fork, by [Nachtzuster](https://github.com/Nachtzuster).
- **[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral)** — the behavioral-analytics engine, by [tomtom215](https://github.com/tomtom215).
- **[fugleramme](https://github.com/arnegiacomo/fugleramme)** — the idea behind the kiosk's plate: a frame that draws the birds heard as nineteenth-century natural-history plates and redraws only when they change, by Arne Giacomo Munthe-Kaas. No code or artwork is shared; its code is MIT and its artwork CC BY-SA 4.0, which this project's licence cannot carry. See [Kiosk Mode](guide/kiosk.md).

## License

Licensed under [**CC BY-NC-SA 4.0**](https://creativecommons.org/licenses/by-nc-sa/4.0/), matching the upstream BirdNET and BirdNET-Pi projects.

> **Non-commercial use only.** You may not use this project to build a commercial product. See [`LICENSE`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/LICENSE) and [`LICENSE-UPSTREAM`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/LICENSE-UPSTREAM) for full terms.

Bird photographs are fetched on demand from Wikipedia and Wikimedia Commons and, where enabled, AviCommons and Flickr, and cached on the station. Each keeps the licence its photographer gave it: they are not all CC BY-SA. Each picture is shown with its title, author, source and licence: under the picture on a species page, in the image API's answer (`/api/v2/species/image/{scientific_name}`, field `credit`), and on the Picture credits page (`/system/credits` on your station), which every page's footer links to. A picture that cannot be credited, or whose licence is not a free one, is not used. The kiosk engraves photographs, which is an adaptation of them, so licences that forbid adaptations are not fetched.

## Related projects

| Repository | Description |
|---|---|
| [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) | ClickHouse-inspired behavioral analytics for DuckDB |
| [quack-rs](https://github.com/tomtom215/quack-rs) | SDK for building DuckDB extensions in Rust |
| [mallardmetrics](https://github.com/tomtom215/mallardmetrics) | Single-binary web analytics (axum + DuckDB) |
| [LyreBirdAudio](https://github.com/tomtom215/LyreBirdAudio) | RTSP audio streaming |

## Contributing

Issues and pull requests are welcome. See [`CONTRIBUTING.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/CONTRIBUTING.md) and the [architecture docs](./reference/architecture.md) to get oriented.
