/* Minimal progressive enhancement (~1 KB gzip). Everything works without it:
   navigation, search and forms are plain HTTP; the theme is a server cookie. */
(function () {
  "use strict";
  var doc = document.documentElement;
  var root = doc;

  /* --- theme toggle, persisted in a cookie so the server can pre-render it --- */
  function setTheme(next) {
    root.dataset.theme = next;
    document.cookie = "rz_theme=" + next + ";path=/;max-age=31536000;samesite=lax";
    var label = document.querySelector("[data-theme-label]");
    if (label) label.textContent = next === "obsidian" ? "Paper" : "Obsidian";
    var meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.content = next === "obsidian" ? "#0b0d0f" : "#efeade";
  }
  document.addEventListener("click", function (e) {
    var btn = e.target.closest("[data-theme-toggle]");
    if (btn) {
      setTheme(root.dataset.theme === "obsidian" ? "paper" : "obsidian");
      return;
    }
    /* --- copy code blocks --- */
    var copy = e.target.closest("[data-copy-btn]");
    if (copy) {
      var frame = copy.closest(".code-frame");
      var code = frame && frame.querySelector("code");
      if (code && navigator.clipboard) {
        navigator.clipboard.writeText(code.innerText).then(function () {
          copy.textContent = "COPIED";
          setTimeout(function () { copy.textContent = "COPY"; }, 1400);
        });
      }
      return;
    }
    /* --- destructive form confirmation --- */
    var submit = e.target.closest("form[data-confirm] button[type=submit]");
    if (submit) {
      var f = submit.closest("form");
      if (!window.confirm(f.dataset.confirm || "Are you sure?")) e.preventDefault();
      return;
    }
    /* --- append an existing tag to the editor --- */
    var tag = e.target.closest("[data-tag]");
    if (tag) {
      var input = document.querySelector('input[name="tags"]');
      if (input) {
        var list = input.value.split(",").map(function (s) { return s.trim(); }).filter(Boolean);
        if (list.indexOf(tag.dataset.tag) < 0) list.push(tag.dataset.tag);
        input.value = list.join(", ");
      }
    }
  });

  /* --- reveal copy buttons only when the API exists --- */
  if (navigator.clipboard) {
    document.querySelectorAll("[data-copy-btn]").forEach(function (b) { b.hidden = false; });
  }

  /* --- editor counters --- */
  var editor = document.querySelector("[data-editor]");
  if (editor) {
    var count = document.querySelector("[data-count]");
    var read = document.querySelector("[data-read]");
    var update = function () {
      var text = editor.value;
      if (count) count.textContent = String(text.length);
      if (read) read.textContent = String(Math.max(1, Math.round(text.split(/\s+/).filter(Boolean).length / 150)));
    };
    editor.addEventListener("input", update);
    update();
    /* Ctrl/Cmd+S saves the form instead of being swallowed by the browser */
    editor.addEventListener("keydown", function (e) {
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        editor.form.submit();
      }
    });
  }

  /* --- terminal-style shortcuts: "/" focuses search, g+h/a/c jumps --- */
  var pendingG = 0;
  document.addEventListener("keydown", function (e) {
    var tag = (e.target.tagName || "").toLowerCase();
    if (tag === "input" || tag === "textarea" || tag === "select" || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === "/") {
      var q = document.querySelector("[data-search-input]");
      if (q) { e.preventDefault(); q.focus(); }
      return;
    }
    if (e.key === "g") { pendingG = Date.now(); return; }
    if (Date.now() - pendingG < 900) {
      var map = { h: "/", a: "/articles", c: "/cabinet" };
      var dest = map[e.key];
      if (dest) { e.preventDefault(); location.href = dest; }
      pendingG = 0;
    }
  });

  /* --- prefetch article links on hover (cheap, one request each) --- */
  if (window.requestIdleCallback) {
    document.addEventListener("mouseover", function (e) {
      var a = e.target.closest('a[href*="/articles/"]');
      if (!a || a.dataset.prefetched || a.target === "_blank") return;
      a.dataset.prefetched = "1";
      var link = document.createElement("link");
      link.rel = "prefetch";
      link.href = a.href;
      document.head.appendChild(link);
    }, { passive: true });
  }
})();
