// Brings the active tab of a home's tab strip into view.
//
// On a phone the strip is wider than the screen and scrolls sideways, and the page you are on can
// be off the screen to the right: the tabs are grouped under When, Who and How, and "Sequences"
// is the last of seven. Nothing then says where you are. This scrolls the strip, once, so that
// the active tab is in its middle (or as near as the strip allows), at once and not animated.
// With no script the strip is as it was served: scrolled to the left.
(function () {
  "use strict";

  function reveal() {
    document.querySelectorAll(".bnb-subtabs").forEach(function (strip) {
      var tab = strip.querySelector(".bnb-subtab.active");
      if (!tab || strip.scrollWidth <= strip.clientWidth) return;
      var tabLeft = tab.getBoundingClientRect().left - strip.getBoundingClientRect().left + strip.scrollLeft;
      strip.scrollLeft = Math.max(0, tabLeft - (strip.clientWidth - tab.offsetWidth) / 2);
    });
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", reveal);
  else reveal();
})();
