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

function makeCaches() {
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
        addAll: async () => {},
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

function boot(network) {
  const listeners = {};
  const caches = makeCaches();
  const sandbox = {
    self: {
      location: { href: `${ORIGIN}/static/sw.js?v=test`, origin: ORIGIN },
      addEventListener: (type, fn) => { listeners[type] = fn; },
      skipWaiting() {},
      clients: { claim() {} },
    },
    caches,
    fetch: network,
    URL, Response, Headers, Promise, console,
  };
  vm.runInNewContext(source, sandbox);
  return { listeners, caches };
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

console.log(JSON.stringify(out));
