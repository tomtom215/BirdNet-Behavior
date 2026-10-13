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
    var t;
    window.addEventListener("resize", function () { clearTimeout(t); t = setTimeout(mark, 150); });
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", mark);
    else mark();
})();
