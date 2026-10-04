/* Mobile navigation toggle (only script on the website). */
(function () {
  var t = document.getElementById("nav-toggle");
  var l = document.getElementById("nav-links");
  if (!t || !l) return;
  t.addEventListener("click", function () {
    var open = l.classList.toggle("open");
    t.setAttribute("aria-expanded", String(open));
  });
})();
