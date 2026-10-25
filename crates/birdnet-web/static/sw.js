// BirdNet-Behavior service worker · O-24.
//
// STATE: DORMANT. This worker is registered in `<base>/static/` (see the
// registration at the foot of templates/layout.html), and a worker controls
// only the pages inside its scope. No page of the dashboard is in `/static/`,
// so `navigator.serviceWorker.controller` is null on every one of them, the
// fetch handler below never receives a page request, and the only thing this
// file does is warm and version a cache of static files that nothing reads.
// (Found by driving Chromium 141, Firefox 157 and WebKit 27.2; the registration
// had no `scope`, whose default is the script's directory, and the server's
// `Service-Worker-Allowed: /` header only permits a wider one.) The goals
// below are therefore what the worker is FOR, not what any release has done:
//
//   1. Never let auth state leak. /admin/*, /login, /logout, /r/*, /feeds/*
//      and anything carrying a Set-Cookie header bypass the worker entirely.
//   2. Make the dashboard feel instant on repeat visits.
//   3. Survive a brief network outage for kiosk-mode wall displays.
//
// Non-goals: PWA-push, background-sync, offline editing of any kind.
//
// Registering it with `{ scope: '/' }` is a one-word change and is NOT safe
// to do as it stands. Run in all three engines with scope `/`, it controls
// pages, answers `/`, `/species` and `/recordings` from cache with the server
// stopped, and does nothing else that the goals above promise:
//
//   * The route table is out of date. Of the 15 paths in SWR_PATHS only `/`,
//     `/species` and `/recordings` still answer 200; the other twelve are
//     308s to `/patterns`, `/reports`, `/station`, `/species?view=…`, which
//     the worker never caches. `/kiosk`, the wall display this exists for, is
//     not listed, and fails with a connection error when the station is down.
//   * `/kiosk` is a standalone document that never registers the worker, and
//     fills itself from `/pages/kiosk-content`, a partial the worker would not
//     cache; a copy of the shell with no numbers is not a display. Neither
//     does it have a cue that what it shows is old ("so it never goes stale"
//     is what the manual says of it).
//   * The page varies with the session. A signed-in browser's `/` carries a
//     "Sign out" form a signed-out one does not, with no `Vary` and no
//     `no-store` on an open station, so a cache keyed by URL hands one the
//     other's chrome, and the footer's "running · …" uptime is text in the copy. A private
//     station is covered (the gate marks its pages `no-store`); an open one
//     is not. Clearing the page cache on every request to /login and /logout
//     would cover the sign-in and sign-out edges, not an expiring session.
//   * `res.headers.has('set-cookie')` below is always false: fetch hides
//     Set-Cookie from a worker as from a page (measured in the three engines),
//     so it was never a defence. Only `no-store` and `redirected` are.
//   * Stale-while-revalidate shows the previous visit's shell first. Today's
//     numbers arrive through uncached partials, so what is stale is chrome,
//     not data; but a returning visitor sees it, and a worker of an older
//     release answers with an older shell until it is replaced.
//   * The routing below (NETWORK_ONLY_PREFIXES, SWR_PATHS, the `/static/`
//     test) compares root-absolute paths. Only the install precache is
//     prefixed with BASE; under BIRDNET_BASE_PATH the fetch handler would
//     match nothing.
//
// Enabling it means fixing those, giving /kiosk a registration and an offline
// cue, and a real-browser gate that exercises the worker at scope `/` in each
// engine. tools/visual-qa/sw-state.mjs holds the line until then: it fails if
// a dashboard page becomes controlled.

const BUILD = (function () {
  // Layout substitutes the build hash into the registration URL as `?v=…`.
  // We pick it up via the URL of this script so the cache version invalidates
  // on every release without us shipping a separate file.
  try {
    return new URL(self.location.href).searchParams.get('v') || 'dev';
  } catch (e) {
    return 'dev';
  }
})();

// The reverse-proxy prefix (BIRDNET_BASE_PATH), taken from where this script was
// served: `/bn/static/sw.js` -> `/bn`, `/static/sw.js` -> ``. The page registers
// it at `<base>/static/sw.js` (templates/layout.html). The install precache must
// ask for `<base>/static/…`: with root-absolute URLs, a station under `/bn`
// failed its install on a 404 (Cache.addAll rejects), which discards the
// registration, after requests to the HOST's `/static/…`, which on a shared host
// belong to something else.
const BASE = (function () {
  try {
    return new URL(self.location.href).pathname.replace(/\/static\/sw\.js$/, '');
  } catch (e) {
    return '';
  }
})();

const STATIC_CACHE = `bnb-static-${BUILD}`;
const DASH_CACHE   = `bnb-dash-${BUILD}`;

// The stylesheets carry the build query the pages link them with, so this
// precache warms the exact URL a page will request. Without it the worker
// cached a *different* URL from the one the document used, and — because
// `Cache.addAll` fetches through the ordinary HTTP cache, which honours
// `immutable` — the entry it warmed could be a year old.
const PRECACHE = [
  `/static/css/app.css?v=${BUILD}`,
  `/static/css/print.css?v=${BUILD}`,
  '/static/htmx.min.js',
  '/static/htmx-sse.js',
  '/static/live-detections.js',
  '/static/theme-guard.js',
  '/static/manifest.webmanifest',
  '/static/icon-192.png',
  '/static/icon-512.png',
];

// Routes that MUST be network-only — auth, signed links, live data, partials.
const NETWORK_ONLY_PREFIXES = [
  '/admin',
  '/login',
  '/logout',
  '/r/',
  '/feeds/',
  '/api/v2/ws',
];

// Routes that are safe to stale-while-revalidate (public, idempotent GET pages).
const SWR_PATHS = new Set([
  '/', '/today', '/species', '/heatmap', '/migration',
  '/correlation', '/timeseries', '/history', '/life-list',
  '/year-in-review', '/weekly', '/recordings', '/gallery',
  '/analytics/dawn-chorus', '/system'
]);

// ── Lifecycle ──────────────────────────────────────────────────────────
self.addEventListener('install', function (event) {
  event.waitUntil(
    caches.open(STATIC_CACHE).then(function (c) { return c.addAll(PRECACHE.map(function (p) { return BASE + p; })); })
      .then(function () { return self.skipWaiting(); })
  );
});

self.addEventListener('activate', function (event) {
  event.waitUntil(
    caches.keys().then(function (keys) {
      return Promise.all(keys.map(function (k) {
        if (k !== STATIC_CACHE && k !== DASH_CACHE) return caches.delete(k);
      }));
    }).then(function () { return self.clients.claim(); })
  );
});

// Posts from the page can ask the worker to skip its install delay. Only a page
// of this origin is a page of this station: a window of another origin that
// obtained a reference to this worker's clients must not be able to promote a
// waiting worker. (A worker's clients are same-origin by construction, so this
// is a check that never fires in practice; it states the assumption in the code
// that depends on it, and tests/sw_harness.mjs holds the line.)
self.addEventListener('message', function (event) {
  if (event.origin !== self.location.origin) return;
  if (event.data && event.data.type === 'SKIP_WAITING') self.skipWaiting();
});

// ── Fetch routing ──────────────────────────────────────────────────────
self.addEventListener('fetch', function (event) {
  const req = event.request;
  if (req.method !== 'GET') return;

  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;          // cross-origin: untouched

  // Hard bypass: auth-sensitive paths.
  if (NETWORK_ONLY_PREFIXES.some(function (p) { return url.pathname.startsWith(p); })) {
    return;                                                  // fall through to network
  }
  // Range requests (audio scrubbing) — let the network handle byte ranges.
  if (req.headers.has('range')) return;

  // Cache-first for static assets.
  if (url.pathname.startsWith('/static/')) {
    event.respondWith(cacheFirst(STATIC_CACHE, req));
    return;
  }

  // Stale-while-revalidate for dashboard surfaces.
  if (SWR_PATHS.has(url.pathname)) {
    event.respondWith(staleWhileRevalidate(DASH_CACHE, req));
    return;
  }
  // Default: pass-through (no caching).
});

// ── Strategies ─────────────────────────────────────────────────────────
function cacheFirst(cacheName, req) {
  return caches.open(cacheName).then(function (cache) {
    return cache.match(req).then(function (cached) {
      if (cached) return cached;
      return fetch(req).then(function (res) {
        if (isCacheable(res)) cache.put(req, res.clone()).catch(function () {});
        return res;
      });
    });
  });
}

function staleWhileRevalidate(cacheName, req) {
  return caches.open(cacheName).then(function (cache) {
    return cache.match(req).then(function (cached) {
      const fetched = fetch(req).then(function (res) {
        if (isCacheable(res)) cache.put(req, res.clone()).catch(function () {});
        return res;
      }).catch(function () { return cached; });
      return cached || fetched;
    });
  });
}

function isCacheable(res) {
  // Never cache redirects, opaque/cross-origin responses, or a response that
  // says not to be kept. (Set-Cookie cannot be tested for here: fetch does not
  // expose it to a worker. The routing rules exclude /admin/* and /login.)
  if (!res || !res.ok) return false;
  if (res.type !== 'basic') return false;
  // A response that was reached through a redirect is not the page asked for.
  // `fetch` follows a `303 /login` and hands back the sign-in form with
  // `ok === true`, so a signed-out request for /today would cache the form
  // under /today — and a signed-in browser would then be shown it first, from
  // cache, every time, until the revalidation landed behind it.
  if (res.redirected) return false;
  if ((res.headers.get('cache-control') || '').includes('no-store')) return false;
  return true;
}
