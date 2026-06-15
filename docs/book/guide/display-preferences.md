# Display Preferences

BirdNet-Behavior adapts to how and where you watch it — a phone on the trail, a wall-mounted Pi touchscreen, a desktop in a bright room. The **Display Preferences** card on the [Station Health](../admin/system.md) tab (`/station`) lets each viewer tune the interface; choices are stored locally in the browser, so they're per-device and need no login.

## What you can change

- **Theme** — Light, Dark, or follow the operating system. Dark mode is a cool "observatory" black tuned so the moss/dawn/rare accents glow.
- **Density** — *Compact*, *Regular*, or *Comfy*. Compact tightens spacing for dense dashboards and small screens; comfy opens it up for touch and across-the-room reading.
- **Motion** — *Reduced* honours users who prefer fewer animations (and is picked up automatically from the OS "reduce motion" setting); the live-feed pulse and chart transitions are quietened.
- **Contrast** — *High* strengthens borders and text contrast for bright environments and accessibility.

## How it's applied (no flash on load)

Preferences are stored under the `theme`, `bnb-density`, `bnb-motion`, and `bnb-contrast` keys in `localStorage`. A tiny guard script runs **before first paint** — both inline in the main layout and as `/static/theme-guard.js` for the standalone admin pages — and sets `data-theme` / `data-motion` / `data-contrast` attributes and the `--density` variable on `<html>` immediately. That pre-paint step is what prevents the brief "flash of the wrong theme" (FOUC) when you reload a page.

Because the values live in the browser, they persist across reloads and survive station restarts, and two people looking at the same station on different devices each get their own settings.
