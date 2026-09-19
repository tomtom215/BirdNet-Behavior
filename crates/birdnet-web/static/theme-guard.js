// Resolve every display preference before first paint.
//
// This is the standalone-page counterpart of the inline guard in
// `templates/layout.html`, and its previous comment claimed the two mirrored
// each other while it read two of the four keys layout.html reads. The effect,
// measured: on `/admin/doctor` (and every other admin page that has not folded
// into a Station tab) `data-motion` and `data-contrast` were both absent, so an
// operator who chose "Reduced" or "High contrast" under Display preferences
// silently lost both on the most form-dense screens in the product — the ones
// where they matter most.
//
// `crates/birdnet-web/tests/the_two_display_preference_guards_agree.rs` now
// fails if either guard stops reading a key the other reads, so the pair
// cannot drift apart again.
(function () {
  var el = document.documentElement;
  var read = function (k) {
    try {
      return localStorage.getItem(k);
    } catch (e) {
      // Private mode, blocked site data: fall through to the OS preference.
      return null;
    }
  };

  var t = read('theme');
  if (t !== 'light' && t !== 'dark') {
    t = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  el.setAttribute('data-theme', t);

  var d = read('bnb-density');
  if (d === 'compact' || d === 'comfy' || d === 'regular') {
    el.style.setProperty('--density', d === 'compact' ? '0.78' : d === 'comfy' ? '1.15' : '1');
  }

  var m = read('bnb-motion');
  if (m === 'reduced') {
    el.setAttribute('data-motion', 'reduced');
  }

  var c = read('bnb-contrast');
  if (c === 'high') {
    el.setAttribute('data-contrast', 'high');
  }
})();
