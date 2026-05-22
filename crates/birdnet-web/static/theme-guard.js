// Resolve theme + density before paint. Mirrors the inline guard in
// layout.html so standalone pages (admin) honour the user's saved theme.
(function () {
  var t = localStorage.getItem('theme');
  if (t !== 'light' && t !== 'dark') {
    t = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  document.documentElement.setAttribute('data-theme', t);
  var d = localStorage.getItem('bnb-density');
  if (d === 'compact' || d === 'comfy' || d === 'regular') {
    document.documentElement.style.setProperty('--density', d === 'compact' ? '0.78' : d === 'comfy' ? '1.15' : '1');
  }
})();
