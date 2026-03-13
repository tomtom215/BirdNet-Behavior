// HTMX SSE Extension (minimal, air-gapped compatible)
// Compatible with HTMX 1.x and 2.x
(function(){
  var api;
  htmx.defineExtension('sse', {
    init: function(a) { api = a; },
    onEvent: function(name, evt) {
      if (name === 'htmx:beforeProcessNode') {
        var elt = evt.detail.elt;
        if (elt.hasAttribute('sse-connect') && !elt._htmxSSE) {
          var url = elt.getAttribute('sse-connect');
          var swap = elt.getAttribute('sse-swap') || 'message';
          var es = new EventSource(url);
          elt._htmxSSE = es;
          es.onopen = function() {
            htmx.trigger(document.body, 'htmx:sseOpen', {source: elt});
          };
          es.onerror = function() {
            htmx.trigger(document.body, 'htmx:sseError', {source: elt});
          };
          es.addEventListener(swap, function(e) {
            var target = elt;
            var existing = elt.innerHTML;
            // Append new content
            var tmp = document.createElement('div');
            tmp.innerHTML = e.data;
            while (tmp.firstChild) elt.appendChild(tmp.firstChild);
            htmx.trigger(document.body, 'htmx:afterSwap', {elt: elt});
          });
          // Cleanup on remove
          api.onElRemoved(elt, function() {
            if (elt._htmxSSE) { elt._htmxSSE.close(); delete elt._htmxSSE; }
          });
        }
      }
    }
  });
})();
