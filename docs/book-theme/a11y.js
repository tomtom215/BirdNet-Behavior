// Make horizontally scrolling regions reachable from the keyboard.
//
// A wide table or code block on a phone scrolls sideways inside its own box,
// and mdBook's markup gives that box no tabindex, so a keyboard user can see
// it is cut off and cannot scroll it (WCAG 2.1.1; axe
// `scrollable-region-focusable`, found on most pages at 390 px). Only boxes
// that actually overflow are made focusable, so the tab order does not fill
// with stops that do nothing; re-checked on resize because overflow depends
// on the viewport.
(function () {
    "use strict";
    function mark() {
        document.querySelectorAll(".content .table-wrapper, .content pre > code").forEach(function (el) {
            var scrolls = el.scrollWidth > el.clientWidth;
            if (scrolls && !el.hasAttribute("tabindex")) {
                // tabindex only: a role="region" with one shared label made
                // every box a duplicate landmark (axe `landmark-unique`).
                el.setAttribute("tabindex", "0");
                el.dataset.bnbScroll = "1";
            } else if (!scrolls && el.dataset.bnbScroll) {
                el.removeAttribute("tabindex");
                delete el.dataset.bnbScroll;
            }
        });
    }
    // mdBook 0.5 draws the table-of-contents toggle as a <label for=…> that
    // carries aria-label and aria-controls, which a label may not have (axe
    // `aria-allowed-attr`, critical, and `aria-prohibited-attr` on every page),
    // and which no key can reach: the checkbox it drives is display:none and the
    // label takes no focus. Giving it the role it is used as, a place in the tab
    // order and Enter/Space makes the markup valid and the control reachable
    // (mdBook's own script already keeps its aria-expanded current).
    function toc() {
        var label = document.getElementById("mdbook-sidebar-toggle");
        var box = document.getElementById("mdbook-sidebar-toggle-anchor");
        if (!label || !box || label.getAttribute("role")) return;
        label.setAttribute("role", "button");
        label.setAttribute("tabindex", "0");
        label.addEventListener("keydown", function (e) {
            if (e.key === "Enter" || e.key === " ") { e.preventDefault(); label.click(); }
        });
    }
    var t;
    window.addEventListener("resize", function () { clearTimeout(t); t = setTimeout(mark, 150); });
    function init() { mark(); toc(); }
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
    else init();
})();
