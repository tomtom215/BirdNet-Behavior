// BirdNet-Behavior service worker · O-24.
//
// Goals (in order):
//   1. Never let auth state leak. /admin/*, /login, /logout, /r/*, /feeds/*
//      and anything carrying a Set-Cookie header bypass the worker entirely.
//   2. Make the dashboard feel instant on repeat visits.
//   3. Survive a brief network outage for kiosk-mode wall displays.
//
// Non-goals: PWA-push, background-sync, offline editing of any kind.

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

const STATIC_CACHE = `bnb-static-${BUILD}`;
const DASH_CACHE   = `bnb-dash-${BUILD}`;

// The stylesheets carry the build query the pages link them with, so this
// precache warms the exact URL a page will request. Without it the worker
// cached a *different* URL from the one the document used, and — because
// `Cache.addAll` fetches through the ordinary HTTP cache, which honours
// `immutable` — the entry it warmed could be a year old.
const PRECACHE = [
  '/',
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
    caches.open(STATIC_CACHE).then(function (c) { return c.addAll(PRECACHE); })
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

// Posts from the page can ask the worker to skip its install delay.
self.addEventListener('message', function (event) {
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
  // Never cache redirects, opaque/cross-origin responses, or anything with
  // Set-Cookie (defence-in-depth — the routing rules already exclude /admin/*).
  if (!res || !res.ok) return false;
  if (res.type !== 'basic') return false;
  if (res.headers.has('set-cookie')) return false;
  if ((res.headers.get('cache-control') || '').includes('no-store')) return false;
  return true;
}
