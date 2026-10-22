// Drives static/sw.js's real fetch handler against stub caches and a scripted
// network, and prints one JSON object describing what it kept and served.
//
//   node sw_harness.mjs <path to sw.js>
//
// Run by `the_service_worker_does_not_keep_a_private_page.rs`. Nothing here
// re-implements the worker: sw.js is evaluated as-is in a `vm` context whose
// only inventions are `self`, `caches` and `fetch`.

import fs from 'node:fs';
import vm from 'node:vm';

const swPath = process.argv[2];
const source = fs.readFileSync(swPath, 'utf8');
const ORIGIN = 'https://station.test';

function makeCaches(precached) {
  const stores = new Map();
  return {
    stores,
    open: async (name) => {
      if (!stores.has(name)) stores.set(name, new Map());
      const entries = stores.get(name);
      const key = (r) => (typeof r === 'string' ? r : r.url);
      return {
        match: async (r) => entries.get(key(r)),
        put: async (r, res) => { entries.set(key(r), res); },
        addAll: async (urls) => { precached.push(...urls); },
      };
    },
    keys: async () => [...stores.keys()],
    delete: async (k) => stores.delete(k),
  };
}

// A same-origin response as the worker sees it from `fetch`: `type: 'basic'`,
// and `redirected` true when the browser followed a redirect to get it.
function basic(body, { headers = {}, redirected = false, status = 200 } = {}) {
  const res = new Response(body, { status, headers });
  Object.defineProperty(res, 'type', { value: 'basic' });
  if (redirected) Object.defineProperty(res, 'redirected', { value: true });
  return res;
}

function boot(network, scriptPath = '/static/sw.js') {
  const listeners = {};
  const precached = [];
  const caches = makeCaches(precached);
  let skips = 0;
  const sandbox = {
    self: {
      location: { href: `${ORIGIN}${scriptPath}?v=test`, origin: ORIGIN },
      addEventListener: (type, fn) => { listeners[type] = fn; },
      skipWaiting() { skips += 1; },
      clients: { claim() {} },
    },
    caches,
    fetch: network,
    URL, Response, Headers, Promise, console,
  };
  vm.runInNewContext(source, sandbox);
  return { listeners, caches, skips: () => skips, precached };
}

// Ask the worker for `path`. Reports whether it took the request on at all, and
// what it answered with.
async function ask(worker, path) {
  const request = { method: 'GET', url: ORIGIN + path, headers: new Headers() };
  let answer;
  worker.listeners.fetch({ request, respondWith: (p) => { answer = p; } });
  if (answer === undefined) return { handled: false };
  try {
    const res = await answer;
    if (res === undefined) return { handled: true, served: false };
    return { handled: true, served: true, body: await res.text() };
  } catch (e) {
    return { handled: true, served: false, error: String(e) };
  }
}

function kept(worker) {
  let n = 0;
  for (const entries of worker.caches.stores.values()) n += entries.size;
  return n;
}

// One scenario: the page is fetched with the network up, then asked for again
// with the network gone. `kept` is what the worker stored; `offline` is what it
// could still answer.
async function scenario(response, path = '/today') {
  let up = true;
  const worker = boot(async () => {
    if (!up) throw new Error('offline');
    return response();
  });
  const online = await ask(worker, path);
  const afterOnline = kept(worker);
  up = false;
  const offline = await ask(worker, path);
  return {
    online: online.served === true,
    kept: afterOnline,
    offline: offline.served === true ? offline.body : null,
    error: online.error,
  };
}

// Deliver a `message` event as the browser would: `origin` is the sender's
// origin, which for a same-origin client is the worker's own. Returns how many
// times the worker called `skipWaiting()` because of it.
function post(worker, data, origin) {
  const before = worker.skips();
  worker.listeners.message({ data, origin });
  return worker.skips() - before;
}

const out = {
  // An open station's page: kept, and served again with the network down.
  public_page: await scenario(() => basic('<p>public dashboard</p>', { headers: { 'content-type': 'text/html' } })),
  // A private station's page, marked as the server now marks it.
  private_page: await scenario(() => basic('<p>private dashboard</p>', { headers: { 'cache-control': 'no-store' } })),
  // The sign-in form a signed-out request for /today was redirected to.
  redirected_page: await scenario(() => basic('<form>sign in</form>', { redirected: true })),
  // A static asset: still cache-first, so the harness is not simply refusing everything.
  static_asset: await scenario(() => basic('body{}', { headers: { 'content-type': 'text/css' } }), '/static/css/extra.css'),
};

// /admin and /login never reach the worker's strategies.
const net = boot(async () => basic('x'));
out.admin_handled = (await ask(net, '/admin/overview')).handled;
out.login_handled = (await ask(net, '/login')).handled;

// `SKIP_WAITING` is the one message the worker acts on. It is honoured from the
// worker's own origin (the page that registered it) and from nowhere else, and
// the worker still ignores a message that is not that one.
out.skip_waiting = {
  from_own_origin: post(net, { type: 'SKIP_WAITING' }, ORIGIN),
  from_another_origin: post(net, { type: 'SKIP_WAITING' }, 'https://other.test'),
  from_no_origin: post(net, { type: 'SKIP_WAITING' }, undefined),
  another_message: post(net, { type: 'SOMETHING_ELSE' }, ORIGIN),
  no_data: post(net, undefined, ORIGIN),
};

// What the install asks the network for. Served from the root, every URL is
// `/static/…`; served from `<base>/static/sw.js` (BIRDNET_BASE_PATH) every one
// is `<base>/static/…`. A URL outside the prefix on a station under one is a
// request to the host, which on a shared host is another application's.
async function installRequests(scriptPath) {
  const worker = boot(async () => basic('x'), scriptPath);
  let done;
  worker.listeners.install({ waitUntil: (p) => { done = p; } });
  await done;
  return worker.precached;
}
out.precache = {
  root: await installRequests('/static/sw.js'),
  under_bn: await installRequests('/bn/static/sw.js'),
};

console.log(JSON.stringify(out));
