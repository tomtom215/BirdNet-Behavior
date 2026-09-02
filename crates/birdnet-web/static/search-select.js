// Select-all for the search results' bulk action bar.
//
// The only interactive behaviour the search page needs beyond HTMX, and it is
// here rather than inline because `script-src` carries no 'unsafe-inline': the
// CSP nonce is applied to the layout's own blocks, and a per-page script is a
// file.
//
// Delegated from `document` and re-bound on nothing: the results list is
// swapped in by HTMX after this file has run, so a direct listener on the
// checkbox would attach to an element that does not exist yet and silently do
// nothing after the first search.
(function () {
  "use strict";

  function toggleAll(master) {
    var form = master.closest("form");
    if (!form) return;
    var boxes = form.querySelectorAll('input[type="checkbox"][name="selected"]');
    for (var i = 0; i < boxes.length; i++) {
      boxes[i].checked = master.checked;
    }
  }

  document.addEventListener("change", function (event) {
    var el = event.target;
    if (!el || !el.getAttribute) return;

    if (el.hasAttribute("data-sr-toggle-all")) {
      toggleAll(el);
      return;
    }

    // Unticking one row must untick "select all", or the header claims a
    // selection the form is not going to post.
    if (el.getAttribute("name") === "selected" && !el.checked) {
      var form = el.closest("form");
      if (!form) return;
      var master = form.querySelector("[data-sr-toggle-all]");
      if (master) master.checked = false;
    }
  });
})();
