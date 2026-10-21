# Display Preferences

BirdNet-Behavior adapts to how and where you watch it — a phone on the trail, a wall-mounted Pi touchscreen, a desktop in a bright room. The **Display Preferences** card on the [Station Health](../admin/system.md) tab (`/station`) lets each viewer tune the interface; choices are stored locally in the browser, so they're per-device and need no login.

## What you can change

- **Theme** — Light, Dark, or follow the operating system. Dark mode is a cool "observatory" black tuned so the moss/dawn/rare accents glow.
- **Density** — *Compact*, *Regular*, or *Comfy*. Compact tightens spacing for dense dashboards and small screens; comfy opens it up for touch and across-the-room reading.
- **Motion** — *Reduced* honours users who prefer fewer animations (and is picked up automatically from the OS "reduce motion" setting); the live-feed pulse and chart transitions are quietened.
- **Contrast** — *High* strengthens borders and text contrast for bright environments and accessibility.

- **Clock** and **Temperature** — *Station* (follow the owner's default, below), or 24-hour / 12-hour and °C / °F for this browser. A 12-hour clock prints `4:31 PM`; midnight is `12:xx AM`. Every time the pages show as text and every temperature goes through one formatter; the station stores and exports Celsius and 24-hour ISO times whatever is shown, and feeds, CSV, JSON and the API are not changed. Not converted: the hour labels, tooltips and readouts inside the charts, which are plain text in SVG and attributes, the admin tables whose times are labelled UTC, and the public share page, which is a document of its own.
- **Listening** — three aids for quiet clips and noisy sites, applied while a clip plays in this browser: **Boost** (up to +24 dB), **Cut low rumble** (a high-pass filter, 20 Hz to 8 kHz, logarithmic, for wind and traffic) and **Even out loud and quiet parts** (a compressor). They apply to every player on every page (the play buttons on Today, the Clips list, the live listen, a clip's own page); the controls also sit in a *Listening aids* fold on a clip's page. All three are off by default and then the clip is played exactly as the browser always did. Nothing is ever changed in the recording or in a download.

  The chain ends in a soft limiter, so the output never passes full scale: measured through the page's own graph on four real clips (BirdNET-Pi's magpie, birdnet-go's tawny owl and soundscape, and a fixture clip) at up to +24 dB with the compressor on, the output peak stayed at or below −0.2 dBFS in all 28 cases; without the limiter the same settings reached +16 dBFS. A large boost on an already loud clip is limited, not clean: use it for quiet recordings. The controls are keyboard-operable, announce their values to a screen reader, and use no animation. Needs a browser with Web Audio; without one they say so and change nothing.

### The station's default

**Settings → General → System & Display → Clock** and **Temperature unit** set what a browser that has not chosen shows (`time_format` `24h` or `12h`, `temperature_unit` `celsius` or `fahrenheit`; served at `GET /api/v2/display`). The page is first printed in the canonical 24-hour, Celsius text and re-printed by `static/display-format.js`; the default is remembered for the tab in `sessionStorage`, so a 12-hour station flashes 24-hour text only on the first page of a tab, and with scripting off the canonical text stands. A test (`the_clock_and_temperature_are_printed_in_one_place`) fails on a page that prints a time or temperature by hand, and `tools/visual-qa/display-format.mjs` holds the browser's formatter to the same table of cases as the Rust one.

## How it's applied (no flash on load)

Preferences are stored under the `theme`, `bnb-density`, `bnb-motion`, and `bnb-contrast` keys in `localStorage` (the listening aids under `bnb-listen`; a browser that blocks storage simply does not remember them). A tiny guard script runs **before first paint** — inline in the main layout, where it sets the `data-theme` / `data-motion` / `data-contrast` attributes and the `--density` variable on `<html>` immediately, and as `/static/theme-guard.js` for the standalone admin pages, which applies only the theme and density keys. That pre-paint step is what prevents the brief "flash of the wrong theme" (FOUC) when you reload a page.

Because the values live in the browser, they persist across reloads and survive station restarts, and two people looking at the same station on different devices each get their own settings.
