// One clip player for every page: any `[data-play-src]` button plays its clip,
// a second click on the same button stops it, and a click on another stops the
// first.
//
// Each page used to bring its own. Today's lived inline in today.html, the
// Clips view has a richer one of its own (a now-playing dock), and Recordings →
// Live had none — its feed rows carry `data-play-src` like every other, so
// every ▶ there did nothing. Buttons inside the Clips view (`#rc-clips`) are
// left to that player.
(function () {
  "use strict";
  var player = null;
  var playingBtn = null;
  // The playing button's own name ("Play Robin, 06:10"), put back when it stops. Each button is
  // named for its clip (UIC-12); writing "Play clip" back would undo that after one listen.
  var playingLabel = null;

  // `data-play-src` is written as a site-root path, and the base-path rewrite
  // covers HTML attributes it knows about, not this one — so the prefix is
  // added here, as the other scripts do from `<body data-base-path>`.
  function withBase(src) {
    var base = (document.body && document.body.dataset.basePath) || '';
    if (!base || src.charAt(0) !== '/' || src.indexOf(base + '/') === 0) return src;
    return base + src;
  }

  function stopCurrent() {
    if (player) { player.pause(); }
    // The label moves with the glyph: aria-label wins over text content, so a
    // playing clip's button would otherwise go on announcing "Play …".
    if (playingBtn) {
      playingBtn.classList.remove('playing');
      playingBtn.textContent = '▶';
      playingBtn.setAttribute('aria-label', playingLabel || 'Play clip');
    }
    playingBtn = null;
    playingLabel = null;
  }

  document.addEventListener('click', function (e) {
    var btn = e.target.closest('[data-play-src]');
    if (!btn || btn.closest('#rc-clips')) return;
    e.preventDefault();
    if (btn === playingBtn) { stopCurrent(); return; }
    stopCurrent();
    if (!player) {
      player = new Audio();
      player.preload = 'none';
      player.addEventListener('ended', stopCurrent);
      player.addEventListener('error', stopCurrent);
    }
    player.src = withBase(btn.getAttribute('data-play-src'));
    // Marked playing before `play()` settles, so a second click in that window
    // stops it rather than starting it again.
    playingBtn = btn;
    playingLabel = btn.getAttribute('aria-label');
    btn.classList.add('playing');
    btn.textContent = '⏸';
    btn.setAttribute('aria-label', /^Play /.test(playingLabel || '') ? 'Pause ' + playingLabel.slice(5) : 'Pause clip');
    player.play().catch(function () { if (playingBtn === btn) stopCurrent(); });
  });
})();
