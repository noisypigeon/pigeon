// Routes the dashed line from GitHub's Actions card, up the left gutter, into Checker; see the
// 2026-11-11 amendment of .claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md.
(function () {
  "use strict";
  function start() {
    var wrap = document.getElementById("rail-wrap");
    var rail = document.getElementById("rail");
    var label = document.getElementById("rail-label");
    var from = document.querySelector("[data-rail-from]");
    var to = document.querySelector("[data-rail-to]");
    if (!wrap || !rail || !label || !from || !to) return;
    var path = rail.querySelector(":scope > path");
    var scroller = to.closest(".scroller");
    function draw() {
      var W = wrap.getBoundingClientRect();
      var A = from.getBoundingClientRect();
      var C = to.getBoundingClientRect();
      var S = scroller ? scroller.getBoundingClientRect() : W;
      // Checker scrolled out of view: no line, and the Actions card says it in words.
      var visible = C.left >= S.left - 1 && C.left <= S.right;
      rail.toggleAttribute("hidden", !visible);
      label.toggleAttribute("hidden", !visible);
      if (!visible) return;
      var gx = -14;
      var sx = A.left - W.left, sy = A.top - W.top + 23;
      var ex = C.left - W.left - 1, ey = C.top - W.top + C.height / 2;
      path.setAttribute("d", "M" + sx.toFixed(1) + " " + sy.toFixed(1) + "H" + gx + "V" + ey.toFixed(1) + "H" + ex.toFixed(1));
      label.style.left = gx + 12 + "px";
      label.style.top = ey + 14 + "px";
    }
    draw();
    window.addEventListener("resize", draw);
    if (scroller) scroller.addEventListener("scroll", draw, { passive: true });
    document.querySelectorAll("details").forEach(function (d) { d.addEventListener("toggle", draw); });
    if (typeof ResizeObserver !== "undefined") new ResizeObserver(draw).observe(wrap);
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(draw);
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
})();
