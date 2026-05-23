# The Dawn Chorus

The dawn-chorus page (`/analytics/dawn-chorus`) shows the daily *rhythm* of your yard: which species sing when, wrapped around a 24-hour clock and anchored to the real sunrise and sunset for your location.

![The dawn chorus page](../images/dawn-chorus.png)

## What you're looking at

- **The polar clock** plots detection activity around a 24-hour dial (midnight at the top, noon at the bottom). Each species is a coloured ribbon whose thickness tracks how often it was heard at that hour, so the pre-dawn build-up and the morning peak read at a glance.
- **Sunrise and sunset markers** are drawn from the station's coordinates, so the chorus lines up against actual first light rather than an arbitrary grid.
- **The right rail** lists the contributing species with their peak hour and total count.

## Setting your location

The sun-time overlay uses, in order of preference:

1. `BNB_STATION_LAT` / `BNB_STATION_LON` — set these in the environment for the most explicit control.
2. `BIRDNET_LATITUDE` / `BIRDNET_LONGITUDE` — the same coordinates used for BirdWeather and the recording scheduler.
3. A conservative `05:30` / `20:00` fallback if neither is configured.

```bash
# Example: a station near Boston, MA
BNB_STATION_LAT=42.3601
BNB_STATION_LON=-71.0589
```

## A note on time zones

Sunrise/sunset are computed and displayed in **UTC**, the same frame the rest of the app uses for detection timestamps (it assumes the station clock is UTC, which is the recommended setup for a fixed listening post). The chorus ribbons and the sun markers therefore share one consistent clock. If your recorder writes filenames in local time, the *shape* of the chorus is still correct, but the hour labels and the sun markers will be offset by your UTC difference — run the station in UTC to keep them aligned.

> Like the [migration](./phenology.md) view, this is built entirely from your own detections — no external data.
