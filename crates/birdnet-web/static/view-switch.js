// A segmented control that swaps which picture of the same data a card shows.
//
//   <div class="seg" data-vs="today-view" hidden role="group" aria-label="View">
//     <button type="button" class="btn active" aria-pressed="true"  data-vs-show="#columns, #columns-note">By the hour</button>
//     <button type="button" class="btn"        aria-pressed="false" data-vs-show="#sky, #sky-note">Every song</button>
//   </div>
//
// Each button names (a CSS selector list) what it shows; whatever the *other* buttons of the group
// name and this one does not is hidden. The control is `hidden` in the markup and shown here, so a
// page with no script keeps the picture it was served and no control that does nothing.
//
// A shown element hears a `bnb:view` event (bubbling), which is how a picture that is expensive
// while it is hidden (a script that fetches or draws) learns it may start; it also stops by itself
// when it is off screen. The choice is remembered per group in localStorage, which may be blocked:
// then the first button is the default and nothing is lost.
(function () {
  "use strict";

  var KEY = "bnb-view:";

  function remembered(group) {
    try {
      return window.localStorage.getItem(KEY + group);
    } catch (e) {
      return null;
    }
  }

  function remember(group, index) {
    try {
      window.localStorage.setItem(KEY + group, String(index));
    } catch (e) {
      /* blocked: the default stands */
    }
  }

  function targets(button) {
    var sel = button.getAttribute("data-vs-show") || "";
    return sel ? Array.prototype.slice.call(document.querySelectorAll(sel)) : [];
  }

  function show(group, buttons, index, save) {
    var wanted = targets(buttons[index]);
    var all = [];
    buttons.forEach(function (b) {
      targets(b).forEach(function (t) {
        if (all.indexOf(t) < 0) all.push(t);
      });
    });
    all.forEach(function (t) {
      t.hidden = wanted.indexOf(t) < 0;
    });
    buttons.forEach(function (b, i) {
      var on = i === index;
      b.classList.toggle("active", on);
      b.setAttribute("aria-pressed", on ? "true" : "false");
    });
    wanted.forEach(function (t) {
      t.dispatchEvent(new CustomEvent("bnb:view", { bubbles: true }));
    });
    if (save) remember(group, index);
  }

  function bind(seg) {
    if (seg.__vs) return;
    seg.__vs = true;
    var group = seg.getAttribute("data-vs");
    var buttons = Array.prototype.slice.call(seg.querySelectorAll("button[data-vs-show]"));
    if (buttons.length < 2) return;
    seg.hidden = false;
    var saved = parseInt(remembered(group), 10);
    var start = saved >= 0 && saved < buttons.length ? saved : 0;
    show(group, buttons, start, false);
    buttons.forEach(function (b, i) {
      b.addEventListener("click", function () {
        show(group, buttons, i, true);
      });
    });
  }

  function boot() {
    document.querySelectorAll(".seg[data-vs]").forEach(bind);
  }

  if (typeof document !== "undefined") {
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", boot);
    else boot();
    document.addEventListener("htmx:afterSwap", boot);
  }
})();
