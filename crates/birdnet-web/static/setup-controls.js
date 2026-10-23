// Setup controls: install the species model from the browser, restart the
// station, and wait for it to come back.
//
// Two things use this, and both are found by attribute so any page can carry
// them:
//
//   [data-cls-host]       holds the classifier panel the server renders
//                         (routes/admin/classifier.rs). Its install and restart
//                         buttons, and its download progress, are driven here;
//                         the words stay on the server and are swapped in.
//   [data-restart-now]    a button that restarts the station (the settings
//                         pages' "waiting for a restart" banner, the wizard's
//                         last step). Next to it, [data-restart-status] is where
//                         the progress is said.
//
// "Back" is decided by a *different process* answering, not by the station
// answering: the old one keeps answering for a moment after the button is
// pressed, so GET /admin/system/boot-id is compared with the id seen before.
(function () {
  "use strict";
  // A page can carry the script twice (a form re-rendered by htmx brings its own
  // tag); two sets of listeners would restart the station twice per click.
  if (window.__bnbSetupControls) { return; }
  window.__bnbSetupControls = true;

  function url(p) {
    if (window.bnbUrl) { return window.bnbUrl(p); }
    var b = (document.body && document.body.dataset.basePath) || "";
    return b && p.charAt(0) === "/" ? b + p : p;
  }

  function get(path) {
    return fetch(url(path), { credentials: "same-origin", cache: "no-store" });
  }
  function post(path) {
    return fetch(url(path), { method: "POST", credentials: "same-origin" });
  }

  // ---- restart and wait ---------------------------------------------------

  // Ask for a restart, then poll until a different process answers. `say`
  // receives what to tell the person at each stage; `done` runs when the new
  // process is up.
  function restartAndWait(say, done) {
    get("/admin/system/boot-id").then(function (r) { return r.ok ? r.text() : ""; })
      .catch(function () { return ""; })
      .then(function (before) {
        return post("/admin/system/service/restart").then(function (r) {
          return r.text().then(function (html) { return { html: html, ok: r.ok }; });
        }).then(function (res) {
          var probe = document.createElement("div");
          probe.innerHTML = res.html;
          if (!probe.querySelector(".ctl-ok")) {
            var why = probe.textContent.replace(/\s+/g, " ").trim();
            say("failed", why || "The station could not restart itself.");
            return;
          }
          say("waiting", "Restarting… this takes a few seconds. This page will update by itself.");
          var tries = 0;
          (function poll() {
            tries += 1;
            get("/admin/system/boot-id").then(function (r) { return r.ok ? r.text() : null; })
              .catch(function () { return null; })
              .then(function (now) {
                if (now && before && now !== before) { say("back", "Back."); done(); return; }
                if (now && !before) { say("back", "Back."); done(); return; }
                if (tries > 90) { say("failed", "The station has not come back after three minutes. Check that it is running."); return; }
                setTimeout(poll, 2000);
              });
          })();
        });
      })
      .catch(function () { say("failed", "The request did not reach the station."); });
  }

  // ---- [data-restart-now] -------------------------------------------------

  document.addEventListener("click", function (e) {
    var t = e.target;
    if (!(t instanceof Element)) { return; }
    var btn = t.closest("[data-restart-now]");
    if (!btn) { return; }
    e.preventDefault();
    var status = btn.parentElement && btn.parentElement.querySelector("[data-restart-status]");
    btn.disabled = true;
    restartAndWait(function (stage, text) {
      if (status) { status.textContent = text; }
      if (stage === "failed") { btn.disabled = false; }
    }, function () { window.location.reload(); });
  });

  // ---- classifier panel ---------------------------------------------------

  function panelOf(host) { return host.querySelector("[data-cls-panel]"); }
  function stateOf(host) { var p = panelOf(host); return p ? p.getAttribute("data-cls-state") : ""; }

  function announce(host) {
    host.dispatchEvent(new CustomEvent("cls:state", {
      bubbles: true,
      detail: { state: stateOf(host) }
    }));
  }

  function swap(host, html) {
    host.innerHTML = html;
    announce(host);
    if (stateOf(host) === "installing") { later(host, 1000); }
  }

  function note(host, text) {
    var old = host.querySelector(".cls-note");
    if (old) { old.remove(); }
    var p = document.createElement("p");
    p.className = "cls-note";
    p.setAttribute("role", "alert");
    p.textContent = text;
    host.appendChild(p);
  }

  var timers = new WeakMap();
  function later(host, ms) {
    var t = timers.get(host);
    if (t) { clearTimeout(t); }
    timers.set(host, setTimeout(function () { refresh(host); }, ms));
  }

  function refresh(host) {
    get("/admin/classifier/panel")
      .then(function (r) {
        if (!r.ok) { throw new Error("status " + r.status); }
        return r.text();
      })
      .then(function (html) { swap(host, html); })
      .catch(function () {
        note(host, "Lost contact with the station. Trying again…");
        later(host, 3000);
      });
  }

  document.addEventListener("click", function (e) {
    var t = e.target;
    if (!(t instanceof Element)) { return; }
    var install = t.closest("[data-cls-install]");
    var restart = t.closest("[data-cls-restart]");
    var host = (install || restart) && t.closest("[data-cls-host]");
    if (!host) { return; }
    e.preventDefault();
    if (install) {
      install.disabled = true;
      post("/admin/classifier/install")
        .then(function (r) { return r.text(); })
        .then(function (html) { swap(host, html); })
        .catch(function () { install.disabled = false; note(host, "The request did not reach the station."); });
      return;
    }
    restart.disabled = true;
    restartAndWait(function (stage, text) {
      var p = panelOf(host);
      var detail = p && p.querySelector(".cls-detail");
      if (stage === "waiting") {
        host.setAttribute("aria-busy", "true");
        if (detail) { detail.textContent = text; }
        var actions = p && p.querySelector(".cls-actions");
        if (actions) { actions.textContent = ""; }
      } else if (stage === "failed") {
        restart.disabled = false;
        host.removeAttribute("aria-busy");
        note(host, text);
      }
    }, function () {
      host.removeAttribute("aria-busy");
      refresh(host);
    });
  });

  function start() {
    document.querySelectorAll("[data-cls-host]").forEach(function (host) {
      announce(host);
      if (stateOf(host) === "installing") { later(host, 1000); }
    });
  }
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start);
  } else {
    start();
  }
})();
