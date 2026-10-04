/* Mobile navigation toggle */
(function () {
  var t = document.getElementById("nav-toggle");
  var l = document.getElementById("nav-links");
  if (!t || !l) return;
  t.addEventListener("click", function () {
    var open = l.classList.toggle("open");
    t.setAttribute("aria-expanded", String(open));
  });
})();

/* Checksum click-to-copy handler */
(function () {
  var codes = document.querySelectorAll(".checksum-code");
  codes.forEach(function (el) {
    el.addEventListener("click", function () {
      var hash = el.getAttribute("data-hash") || el.textContent;
      if (!hash) return;
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(hash).then(function () {
          showCopied(el, hash);
        });
      } else {
        var input = document.createElement("input");
        input.value = hash;
        document.body.appendChild(input);
        input.select();
        document.execCommand("copy");
        document.body.removeChild(input);
        showCopied(el, hash);
      }
    });
  });

  function showCopied(el, original) {
    el.textContent = "Copied to clipboard!";
    el.classList.add("copied");
    setTimeout(function () {
      el.textContent = original;
      el.classList.remove("copied");
    }, 2000);
  }
})();

