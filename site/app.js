/* lain project site — tiny progressive enhancements: mobile nav + copy buttons. */
(function () {
  "use strict";

  document.documentElement.classList.add("js");

  var nav = document.getElementById("site-nav");
  var toggle = document.querySelector(".nav-toggle");

  function setNav(open) {
    if (!nav || !toggle) return;
    nav.classList.toggle("open", open);
    toggle.setAttribute("aria-expanded", open ? "true" : "false");
  }

  if (nav && toggle) {
    toggle.addEventListener("click", function () {
      setNav(!nav.classList.contains("open"));
    });
    nav.addEventListener("click", function (event) {
      if (event.target.closest("a")) setNav(false);
    });
    document.addEventListener("keydown", function (event) {
      if (event.key === "Escape" && nav.classList.contains("open")) {
        setNav(false);
        toggle.focus();
      }
    });
    document.addEventListener("click", function (event) {
      if (!nav.classList.contains("open")) return;
      if (event.target.closest && event.target.closest(".site-header")) return;
      setNav(false);
    });
  }

  var status = document.getElementById("copy-status");

  function announce(message) {
    if (status) status.textContent = message;
  }

  function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) {
      return navigator.clipboard.writeText(text);
    }
    return Promise.reject();
  }

  function fallbackCopy(node) {
    var selection = window.getSelection();
    if (!selection) return false;
    var range = document.createRange();
    range.selectNodeContents(node);
    selection.removeAllRanges();
    selection.addRange(range);
    var ok = false;
    try {
      ok = document.execCommand("copy");
    } catch (error) {
      ok = false;
    }
    selection.removeAllRanges();
    return ok;
  }

  function flash(button) {
    var label = button.textContent;
    button.textContent = "copied";
    button.classList.add("ok");
    announce("Copied to clipboard.");
    window.setTimeout(function () {
      button.textContent = label;
      button.classList.remove("ok");
      announce("");
    }, 1600);
  }

  Array.prototype.forEach.call(document.querySelectorAll("[data-copy]"), function (button) {
    button.addEventListener("click", function () {
      var node = document.getElementById(button.getAttribute("data-copy"));
      if (!node) return;
      var text = node.textContent.trim();
      copyText(text).then(
        function () { flash(button); },
        function () {
          if (fallbackCopy(node)) flash(button);
          else announce("Copy failed — select the code and copy manually.");
        }
      );
    });
  });
})();
