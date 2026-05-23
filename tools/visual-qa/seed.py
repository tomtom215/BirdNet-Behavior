#!/usr/bin/env python3
"""Deterministic demo seed for BirdNet-Behavior visual QA / docs screenshots.

Populates a migrated SQLite database with a year-and-a-half of realistic
detections so every screen has rich, consistent data. Anchored to the system
date (= "today"); times are UTC so the dawn-chorus page (which computes sun
times in UTC) lines up with detection clock times.

Usage:
    # 1. let the binary migrate a fresh DB, then:
    python3 seed.py /path/to/birds.db
    # 2. restart the server so the SQLite -> DuckDB sync picks up the rows.

Community: a southern-England garden/reserve (lat 51.48, lon -0.13):
  - residents (year-round, spring song bump)
  - passage migrants with SHARP gaussian peaks (drive the /migration ridges)
  - winter visitors, nocturnal owls
  - one true first-of-year (Spotted Flycatcher: first ever this year)
  - one "overdue" species (Common Nightingale: last year only, ~now)
"""
import math
import random
import sqlite3
import sys
from datetime import date, datetime, timedelta, timezone

DB = sys.argv[1] if len(sys.argv) > 1 else "birds.db"
random.seed(42)

TODAY = datetime.now(timezone.utc).date()
DAYS_BACK = 568  # back to ~2024-11 => full prior year for clean YoY
START = TODAY - timedelta(days=DAYS_BACK)
LAT, LON = 51.48, -0.13

SUNRISE_H = {1: 8.0, 2: 7.2, 3: 6.2, 4: 5.0, 5: 4.2, 6: 3.8,
             7: 4.1, 8: 4.9, 9: 5.7, 10: 6.5, 11: 7.4, 12: 8.1}


def gauss(x, mu, sigma):
    return math.exp(-((x - mu) ** 2) / (2 * sigma * sigma))


def doy(d):
    return d.timetuple().tm_yday


# (common, sci, kind, base_rate/day, params)
SPECIES = [
    ("Eurasian Magpie", "Pica pica", "resident", 3.2, {}),
    ("European Robin", "Erithacus rubecula", "resident", 4.0, {}),
    ("Eurasian Blackbird", "Turdus merula", "resident", 4.6, {}),
    ("Blue Tit", "Cyanistes caeruleus", "resident", 3.8, {}),
    ("Great Tit", "Parus major", "resident", 3.6, {}),
    ("European Goldfinch", "Carduelis carduelis", "resident", 2.6, {}),
    ("Common Chaffinch", "Fringilla coelebs", "resident", 2.8, {}),
    ("Eurasian Wren", "Troglodytes troglodytes", "resident", 3.0, {}),
    ("Dunnock", "Prunella modularis", "resident", 1.9, {}),
    ("European Greenfinch", "Chloris chloris", "resident", 1.4, {}),
    ("Common Wood Pigeon", "Columba palumbus", "resident", 3.4, {}),
    ("Eurasian Collared Dove", "Streptopelia decaocto", "resident", 1.6, {}),
    ("House Sparrow", "Passer domesticus", "resident", 3.1, {}),
    ("Long-tailed Tit", "Aegithalos caudatus", "resident", 1.5, {}),
    ("Eurasian Jay", "Garrulus glandarius", "resident", 1.1, {}),
    ("Carrion Crow", "Corvus corone", "resident", 2.2, {}),
    ("Eurasian Nuthatch", "Sitta europaea", "resident", 1.2, {}),
    ("Coal Tit", "Periparus ater", "resident", 1.0, {}),
    ("Goldcrest", "Regulus regulus", "resident", 0.8, {}),
    ("Song Thrush", "Turdus philomelos", "resident", 2.0, {}),
    ("Common Chiffchaff", "Phylloscopus collybita", "passage", 0.35,
     {"spring": 74, "spring_amp": 4.2, "fall": 255, "fall_amp": 1.8, "sigma": 4.0}),
    ("Eurasian Blackcap", "Sylvia atricapilla", "passage", 0.4,
     {"spring": 92, "spring_amp": 4.6, "fall": 258, "fall_amp": 2.0, "sigma": 4.0}),
    ("Willow Warbler", "Phylloscopus trochilus", "passage", 0.3,
     {"spring": 98, "spring_amp": 3.6, "fall": 248, "fall_amp": 1.6, "sigma": 3.5}),
    ("Common Whitethroat", "Curruca communis", "passage", 0.25,
     {"spring": 110, "spring_amp": 3.2, "fall": 250, "fall_amp": 1.2, "sigma": 3.5}),
    ("Garden Warbler", "Sylvia borin", "passage", 0.2,
     {"spring": 116, "spring_amp": 2.6, "sigma": 3.5}),
    ("Common Cuckoo", "Cuculus canorus", "passage", 0.18,
     {"spring": 108, "spring_amp": 3.0, "sigma": 4.5}),
    ("Common Swift", "Apus apus", "passage", 0.5,
     {"spring": 124, "spring_amp": 5.0, "fall": 224, "fall_amp": 2.2, "sigma": 5.0}),
    ("Barn Swallow", "Hirundo rustica", "passage", 0.45,
     {"spring": 106, "spring_amp": 4.4, "fall": 262, "fall_amp": 2.6, "sigma": 5.0}),
    ("House Martin", "Delichon urbicum", "passage", 0.3,
     {"spring": 116, "spring_amp": 3.0, "fall": 258, "fall_amp": 1.8, "sigma": 4.5}),
    ("Common Redstart", "Phoenicurus phoenicurus", "passage", 0.12,
     {"spring": 104, "spring_amp": 1.8, "fall": 252, "fall_amp": 1.2, "sigma": 3.5}),
    ("Redwing", "Turdus iliacus", "winter", 1.0,
     {"arrive": 296, "depart": 78, "amp": 2.4}),
    ("Fieldfare", "Turdus pilaris", "winter", 0.8,
     {"arrive": 300, "depart": 74, "amp": 2.0}),
    ("Tawny Owl", "Strix aluco", "nocturnal", 0.9, {}),
    ("Barn Owl", "Tyto alba", "nocturnal", 0.5, {}),
    ("Spotted Flycatcher", "Muscicapa striata", "passage", 0.16,
     {"spring": 128, "spring_amp": 2.4, "sigma": 3.5, "only_year": TODAY.year}),
    ("Common Nightingale", "Luscinia megarhynchos", "passage", 0.22,
     {"spring": 140, "spring_amp": 3.4, "sigma": 4.0, "only_year": TODAY.year - 1}),
    ("Eurasian Hobby", "Falco subbuteo", "passage", 0.05,
     {"spring": 118, "spring_amp": 0.8, "sigma": 6.0}),
]


def seasonal_rate(kind, base, p, d):
    n = doy(d)
    if "only_year" in p and d.year != p["only_year"]:
        return 0.0
    if kind == "resident":
        return base * (0.75 + 0.7 * gauss(n, 135, 34))
    if kind == "nocturnal":
        return base * (0.8 + 0.4 * gauss(n, 90, 50))
    if kind == "passage":
        sigma = p.get("sigma", 3.5)
        r = base
        if "spring" in p:
            r += p["spring_amp"] * gauss(n, p["spring"], sigma)
            if 0 <= (n - p["spring"]) <= 70:
                r += base * 1.6
        if "fall" in p:
            r += p["fall_amp"] * gauss(n, p["fall"], sigma + 1)
        return r
    if kind == "winter":
        arrive, depart, amp = p["arrive"], p["depart"], p["amp"]
        if not ((n >= arrive) or (n <= depart)):
            return 0.0
        r = base + amp * gauss(n, arrive + 6, 7)
        r += (amp * 0.7) * gauss((n + 10) % 366, depart % 366, 9)
        return r
    return base


def hour_weights(d, nocturnal):
    sr = SUNRISE_H[d.month]
    w = []
    for h in range(24):
        if nocturnal:
            dist = min(abs(h - 0), abs(h - 24))
            val = 0.15 + 1.0 * gauss(dist, 0, 2.4)
            if 6 <= h <= 18:
                val *= 0.05
        else:
            dawn = 1.0 * gauss(h, sr + 0.6, 1.4)
            aft = 0.4 * gauss(h, 16.5, 2.2)
            base = 0.06 if (sr - 1) <= h <= 20 else 0.005
            val = dawn + aft + base
        w.append(max(val, 0.0001))
    return w


def sample_hour(weights):
    tot = sum(weights)
    r = random.random() * tot
    acc = 0.0
    for h, wv in enumerate(weights):
        acc += wv
        if r <= acc:
            return h
    return 23


def conf_for(kind):
    lo, hi = {"resident": (0.62, 0.94), "passage": (0.45, 0.9),
              "winter": (0.5, 0.88), "nocturnal": (0.55, 0.9)}[kind]
    return round(random.triangular(lo, hi, (lo + hi) / 2 + 0.1), 4)


def poisson(lam):
    if lam <= 0:
        return 0
    el, k, pr = math.exp(-lam), 0, 1.0
    while True:
        k += 1
        pr *= random.random()
        if pr <= el:
            return k - 1
        if k > 60:
            return 60


def main():
    conn = sqlite3.connect(DB)
    cur = conn.cursor()
    cur.execute("PRAGMA journal_mode=WAL")
    for t in ("detections", "quarantine", "notification_log",
              "alert_rules", "species_thresholds"):
        try:
            cur.execute(f"DELETE FROM {t}")
        except sqlite3.OperationalError:
            pass
    conn.commit()

    rows = []
    for i in range(DAYS_BACK + 1):
        d = START + timedelta(days=i)
        ds, wk = d.isoformat(), d.isocalendar()[1]
        for (com, sci, kind, base, p) in SPECIES:
            lam = seasonal_rate(kind, base, p, d)
            if lam <= 0:
                continue
            count = poisson(lam * random.uniform(0.7, 1.3))
            if count <= 0:
                continue
            hw = hour_weights(d, kind == "nocturnal")
            for _ in range(count):
                h, mi, se = sample_hour(hw), random.randint(0, 59), random.randint(0, 59)
                ts = f"{h:02d}:{mi:02d}:{se:02d}"
                c = conf_for(kind)
                locked = 1 if (c > 0.92 and random.random() < 0.04) else 0
                rows.append((ds, ts, sci, com, c, LAT, LON, 0.0, wk, 1.25, 0.0,
                             f"{ds}-birdnet-{ts}.wav", locked, 0.0, None))

    hero_ts = "05:14:08"
    rows.append((TODAY.isoformat(), hero_ts, "Pica pica", "Eurasian Magpie",
                 0.948, LAT, LON, 0.0, TODAY.isocalendar()[1], 1.25, 0.0,
                 f"{TODAY.isoformat()}-birdnet-{hero_ts}.wav", 0, 0.0, "demo-hero-0001"))

    seen, uniq = set(), []
    for r in rows:
        key = (r[0], r[1], r[2], r[11], r[13])
        if key not in seen:
            seen.add(key)
            uniq.append(r)
    cur.executemany(
        "INSERT OR IGNORE INTO detections (Date,Time,Sci_Name,Com_Name,Confidence,"
        "Lat,Lon,Cutoff,Week,Sens,Overlap,File_Name,is_locked,chunk_offset_secs,"
        "correlation_id) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", uniq)

    def dd(n):
        return (TODAY - timedelta(days=n)).isoformat()
    q = [
        (TODAY.isoformat(), "06:42:11", "Falco subbuteo", "Eurasian Hobby", 0.38, 0.21,
         "below_sf_thresh", 0, 0, f"{TODAY.isoformat()}-birdnet-06:42:11.wav", LAT, LON, TODAY.isocalendar()[1]),
        (dd(1), "19:58:03", "Phoenicurus phoenicurus", "Common Redstart", 0.41, 0.33,
         "low_confidence", 0, 0, None, LAT, LON, 1),
        (dd(2), "04:51:47", "Luscinia megarhynchos", "Common Nightingale", 0.44, 0.29,
         "below_sf_thresh", 0, 0, None, LAT, LON, 1),
        (dd(3), "13:02:55", "Accipiter nisus", "Eurasian Sparrowhawk", 0.52, 0.40,
         "manual", 1, 1, None, LAT, LON, 1),
        (dd(4), "07:14:22", "Regulus ignicapilla", "Common Firecrest", 0.47, 0.36,
         "low_confidence", 1, 0, None, LAT, LON, 1),
        (dd(6), "05:33:09", "Locustella naevia", "Common Grasshopper Warbler", 0.39, 0.18,
         "below_sf_thresh", 0, 0, None, LAT, LON, 1),
    ]
    cur.executemany(
        "INSERT OR IGNORE INTO quarantine (date,time,sci_name,com_name,confidence,"
        "sf_probability,reason,reviewed,approved,file_name,lat,lon,week) "
        "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)", q)

    chans = ["apprise", "mqtt", "birdweather"]
    notif = []
    for k in range(14):
        ddate = dd(k // 2)
        com, sci = random.choice([("Common Cuckoo", "Cuculus canorus"),
                                  ("Common Swift", "Apus apus"),
                                  ("Spotted Flycatcher", "Muscicapa striata"),
                                  ("Eurasian Hobby", "Falco subbuteo"),
                                  ("Barn Owl", "Tyto alba")])
        st = random.choices(["sent", "sent", "sent", "failed", "skipped"], k=1)[0]
        notif.append((f"{ddate} {random.randint(4,9):02d}:{random.randint(0,59):02d}:00",
                      chans[k % 3], com, sci, round(random.uniform(0.7, 0.96), 3),
                      ddate, f"{random.randint(4,9):02d}:30:00", st,
                      f"{com} detected ({random.randint(70,96)}%)",
                      "SMTP timeout" if st == "failed" else None))
    cur.executemany(
        "INSERT INTO notification_log (sent_at,channel,species_com_name,"
        "species_sci_name,confidence,detection_date,detection_time,status,message,error) "
        "VALUES (?,?,?,?,?,?,?,?,?,?)", notif)

    cur.executemany(
        "INSERT INTO alert_rules (name,enabled,species_pattern,confidence_min,"
        "confidence_max,hour_start,hour_end,days_of_week,action_type,"
        "action_webhook_url,action_webhook_method,action_webhook_body) "
        "VALUES (?,?,?,?,?,?,?,?,?,?,?,?)", [
            ("Rare visitor webhook", 1, "Hobby|Nightingale|Firecrest", 0.6, 1.0,
             None, None, None, "webhook", "https://hooks.example.org/birds", "POST",
             '{"text":"Rare bird"}'),
            ("Owl night log", 1, "Owl", 0.5, 1.0, 21, 5, None, "log", None, "POST", None),
            ("Suppress low-confidence Wood Pigeon", 1, "Wood Pigeon", 0.0, 0.5,
             None, None, None, "suppress", None, "POST", None),
        ])
    cur.executemany(
        "INSERT OR REPLACE INTO species_thresholds (sci_name,confidence_threshold) "
        "VALUES (?,?)", [("Falco subbuteo", 0.65), ("Cuculus canorus", 0.55),
                         ("Muscicapa striata", 0.6), ("Columba palumbus", 0.45)])
    conn.commit()

    cur.execute("SELECT COUNT(*), COUNT(DISTINCT Com_Name), MIN(Date), MAX(Date) FROM detections")
    total, nsp, mn, mx = cur.fetchone()
    print(f"detections={total} species={nsp} range={mn}..{mx}")
    conn.close()


if __name__ == "__main__":
    main()
