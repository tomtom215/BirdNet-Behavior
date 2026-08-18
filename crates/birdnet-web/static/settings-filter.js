// Filter the admin settings page by typing.
//
// The page carries 54 controls across eight sections and roughly five screens.
// The Station tabs already own task-scoped access to the same sections, so this
// page's job is the opposite one: hold everything at once and let you find it.
// Collapsing the sections would have served the scroll length at the cost of
// that job — and of the browser's own Ctrl+F, which stops matching text inside
// a closed <details> in most engines.
//
// Filtering keeps every setting present and reachable while narrowing what is
// on screen, so it buys the same relief without hiding anything permanently.
//
// Degradation: the markup ships with `hidden` and this script removes it, so a
// browser with JavaScript off shows no dead control and the page behaves
// exactly as it did before — every section expanded.
(function () {
  "use strict";

  var box = document.querySelector(".set-filter");
  var input = document.getElementById("set-filter");
  var status = document.querySelector(".set-filter__count");
  var sections = Array.prototype.slice.call(
    document.querySelectorAll("section.card[id^='set-']")
  );
  if (!box || !input || !sections.length) return;

  // Index links, keyed by the section they point at, so a filtered-out section
  // is dimmed in the jump list rather than becoming a link to nothing.
  var links = {};
  Array.prototype.forEach.call(
    document.querySelectorAll(".set-index__list a"),
    function (a) {
      var href = a.getAttribute("href") || "";
      if (href.charAt(0) === "#") links[href.slice(1)] = a;
    }
  );

  // Each section's searchable text, computed once: its heading, every label,
  // every hint, and the control names, so "rtsp" finds the field whose visible
  // label says "RTSP URL" and "sf_thresh" finds it by the key you read in a
  // config file.
  var haystacks = sections.map(function (s) {
    var names = Array.prototype.map
      .call(s.querySelectorAll("[name]"), function (el) {
        return el.getAttribute("name");
      })
      .join(" ");
    return (s.textContent + " " + names).toLowerCase();
  });

  function apply() {
    var q = input.value.trim().toLowerCase();
    var shown = 0;
    sections.forEach(function (s, i) {
      var hit = q === "" || haystacks[i].indexOf(q) !== -1;
      s.hidden = !hit;
      if (hit) shown++;
      var link = links[s.id];
      if (link) {
        link.classList.toggle("is-filtered-out", !hit);
        // A link to a hidden section is not a useful tab stop.
        if (hit) link.removeAttribute("tabindex");
        else link.setAttribute("tabindex", "-1");
      }
    });
    if (!status) return;
    if (q === "") status.textContent = "";
    else if (shown === 0) status.textContent = "No settings match “" + input.value + "”.";
    else status.textContent = shown + " of " + sections.length + " sections match.";
  }

  box.hidden = false;
  input.addEventListener("input", apply);
  input.addEventListener("keydown", function (e) {
    if (e.key === "Escape" && input.value !== "") {
      input.value = "";
      apply();
    }
  });
  apply();
})();
