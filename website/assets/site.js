// Glance download site: picks the download source for the visitor, asks
// GitHub and Gitee for the latest release (both APIs answer cross-origin
// requests), and points the buttons straight at the installer. Without
// JavaScript, or when an API doesn't answer, the links stay on the release
// pages and the page keeps the version written into it.
(function () {
  "use strict";

  var REPO = "lulu-loopp/glance";
  var SOURCES = {
    github: {
      name: "GitHub",
      api: "https://api.github.com/repos/" + REPO + "/releases?per_page=6",
      page: "https://github.com/" + REPO + "/releases/latest",
      all: "https://github.com/" + REPO + "/releases"
    },
    gitee: {
      name: "Gitee",
      api: "https://gitee.com/api/v5/repos/" + REPO + "/releases?per_page=6&page=1&direction=desc",
      page: "https://gitee.com/" + REPO + "/releases",
      all: "https://gitee.com/" + REPO + "/releases"
    }
  };
  var INSTALLER = /^Glance_[0-9][0-9A-Za-z.\-]*_x64-setup\.exe$/;
  var SHOWN_RELEASES = 4;

  var root = document.documentElement;
  var lang = root.lang.indexOf("zh") === 0 ? "zh" : "en";

  // A language chosen with the switch is kept: the Chinese page then no
  // longer sends a browser that reads no Chinese to the English one (lang.js).
  document.querySelectorAll("a[hreflang]").forEach(function (a) {
    a.addEventListener("click", function () {
      try { localStorage.setItem("glance-lang", a.hreflang.indexOf("zh") === 0 ? "zh" : "en"); } catch (e) { /* not kept: the browser's languages decide */ }
    });
  });

  var TEXT = {
    zh: {
      meta: function (v, size) { return v + " 版 · 免费" + (size ? " · " + size : ""); },
      copied: "已复制，去电脑上打开吧",
      copyFailed: "没能自动复制，请长按上面的网址手动复制",
      more: "展开", less: "收起", latest: "最新",
      date: function (d) { return d.getFullYear() + " 年 " + (d.getMonth() + 1) + " 月 " + d.getDate() + " 日"; }
    },
    en: {
      meta: function (v, size) { return "Version " + v + " · free" + (size ? " · " + size : ""); },
      copied: "Copied. Open it on your PC",
      copyFailed: "Couldn't copy. Press and hold the address above to copy it",
      more: "Show", less: "Hide", latest: "Latest",
      date: function (d) {
        var m = ["January", "February", "March", "April", "May", "June", "July", "August",
          "September", "October", "November", "December"];
        return d.getDate() + " " + m[d.getMonth()] + " " + d.getFullYear();
      }
    }
  }[lang];

  // Visitors whose browser speaks Chinese download from the Gitee mirror,
  // others from GitHub; whichever of the two this browser cannot reach gives
  // way to the other.
  var primary = (navigator.language || "").toLowerCase().indexOf("zh") === 0 ? "gitee" : "github";
  var reached = { github: null, gitee: null };

  // ---- which device is this? ----
  var ua = navigator.userAgent || "";
  var isPhone = /Android|iPhone|iPad|iPod|HarmonyOS|Mobile/i.test(ua) ||
    (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1);
  var isWindows = /Windows NT/.test(ua);
  if (isPhone) root.classList.add("is-phone");
  else if (!isWindows) root.classList.add("is-otheros");

  // ---- download links ----
  var state = { version: null, size: null, url: { github: null, gitee: null } };

  function each(sel, fn) { Array.prototype.forEach.call(document.querySelectorAll(sel), fn); }

  function renderLinks() {
    each("[data-dl=\"primary\"]", function (a) { a.href = state.url[primary] || SOURCES[primary].page; });
    each("[data-dl=\"all\"]", function (a) { a.href = SOURCES[primary].all; });
    if (state.version) {
      each("[data-meta]", function (el) {
        // The size written into the page belongs to the version written there.
        var size = state.size ||
          (state.version === el.getAttribute("data-version") ? el.getAttribute("data-size") : null);
        el.textContent = TEXT.meta(state.version, size);
      });
    }
  }

  function formatSize(bytes) { return (bytes / 1048576).toFixed(1) + " MB"; }

  function fetchJSON(url) {
    var ctl = "AbortController" in window ? new AbortController() : null;
    var timer = ctl && setTimeout(function () { ctl.abort(); }, 8000);
    return fetch(url, { signal: ctl && ctl.signal, headers: { Accept: "application/json" } })
      .then(function (r) {
        if (!r.ok) throw new Error(url + ": " + r.status);
        return r.json();
      })
      .finally(function () { if (timer) clearTimeout(timer); });
  }

  function releasesOf(list) {
    return list
      .filter(function (r) { return !r.draft && !r.prerelease; })
      .sort(function (a, b) {
        return Date.parse(b.published_at || b.created_at) - Date.parse(a.published_at || a.created_at);
      });
  }

  var changelogFrom = null;

  function take(source, list) {
    var releases = releasesOf(list);
    if (!releases.length) return;
    var latest = releases[0];
    var asset = (latest.assets || []).filter(function (a) { return INSTALLER.test(a.name); })[0];
    var version = latest.tag_name.replace(/^v/, "");
    if (asset) state.url[source] = asset.browser_download_url;
    if ((source === primary || !state.version) && state.version !== version) {
      state.version = version;
      state.size = null;
    }
    if (asset && asset.size && version === state.version) state.size = formatSize(asset.size);
    renderLinks();
    // GitHub keeps the whole history; Gitee mirrors it from 0.1.6 on.
    if (changelogFrom !== "github") {
      changelogFrom = source;
      renderChangelog(releases.slice(0, SHOWN_RELEASES));
    }
  }

  renderLinks();
  if (window.fetch) {
    ["github", "gitee"].forEach(function (s) {
      fetchJSON(SOURCES[s].api).then(function (list) {
        reached[s] = true;
        if (reached[primary] === false) primary = s;
        take(s, list);
      }, function () {
        reached[s] = false;
        var other = s === "github" ? "gitee" : "github";
        if (s === primary && reached[other]) { primary = other; renderLinks(); }
      });
    });
  }

  // ---- release notes ----
  // A release's notes hold an English and a Chinese half, split by a "---" line.
  function notesFor(body) {
    var parts = body.replace(/\r\n?/g, "\n").split(/\n-{3,}[ \t]*\n/);
    var cjk = /[一-鿿]/;
    for (var i = 0; i < parts.length; i++) {
      if (cjk.test(parts[i]) === (lang === "zh")) return parts[i].trim();
    }
    return parts[0].trim();
  }

  function esc(s) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  }

  function inline(s) {
    return esc(s)
      .replace(/`([^`]+)`/g, "<code>$1</code>")
      .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
      .replace(/https?:\/\/[^\s<]*[^\s<.,;:!?)。，；：）]/g, function (u) {
        return "<a href=\"" + u + "\" rel=\"noopener\">" + u + "</a>";
      });
  }

  // The notes use a small part of Markdown: bold lines as headings, "- " lists, paragraphs.
  function markdown(md) {
    var out = [], list = false;
    md.split("\n").forEach(function (line) {
      var t = line.trim();
      var item = /^[-*] +(.*)$/.exec(t);
      if (item) {
        if (!list) { out.push("<ul>"); list = true; }
        out.push("<li>" + inline(item[1]) + "</li>");
        return;
      }
      if (list) { out.push("</ul>"); list = false; }
      if (!t) return;
      var head = /^\*\*([^*]+)\*\*:?$/.exec(t);
      out.push(head ? "<h4>" + esc(head[1]) + "</h4>" : "<p>" + inline(t) + "</p>");
    });
    if (list) out.push("</ul>");
    return out.join("");
  }

  function renderChangelog(releases) {
    var box = document.querySelector("[data-changelog]");
    if (!box) return;
    box.innerHTML = releases.map(function (r, i) {
      var d = new Date(Date.parse(r.published_at || r.created_at));
      var head = "<span class=\"head\" data-more=\"" + TEXT.more + "\" data-less=\"" + TEXT.less + "\">" +
        "<span class=\"v\">" + esc(r.tag_name.replace(/^v/, "")) + "</span>" +
        "<span class=\"d\">" + TEXT.date(d) + "</span>" +
        (i === 0 ? "<span class=\"tag\">" + TEXT.latest + "</span>" : "") + "</span>";
      var notes = "<div class=\"notes\">" + markdown(notesFor(r.body || "")) + "</div>";
      return i === 0
        ? "<article class=\"release\">" + head + notes + "</article>"
        : "<details class=\"release\"><summary>" + head + "</summary>" + notes + "</details>";
    }).join("");
  }

  // ---- phone: copy this page's address ----
  var pageUrl = location.origin + location.pathname;
  each("[data-url]", function (el) { el.textContent = pageUrl; });

  function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) {
      return navigator.clipboard.writeText(text).catch(function () { return legacyCopy(text); });
    }
    return legacyCopy(text);
  }

  // In-app browsers (Xiaohongshu, WeChat) often lack the clipboard API.
  function legacyCopy(text) {
    var ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.top = "0";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    ta.setSelectionRange(0, text.length);
    var ok = false;
    try { ok = document.execCommand("copy"); } catch (e) { ok = false; }
    document.body.removeChild(ta);
    return ok ? Promise.resolve() : Promise.reject(new Error("copy"));
  }

  each("[data-copy]", function (btn) {
    var label = btn.textContent;
    btn.addEventListener("click", function () {
      copyText(pageUrl).then(function () {
        btn.textContent = TEXT.copied;
      }, function () {
        btn.textContent = TEXT.copyFailed;
        var url = btn.parentNode.querySelector("[data-url]");
        if (url) window.getSelection().selectAllChildren(url);
      }).then(function () {
        setTimeout(function () { btn.textContent = label; }, 3000);
      });
    });
  });

  // ---- the site's own count (see _worker.js) ----
  // The page shown, and the installer taken: what, from where, in which
  // language; nothing that tells who. A browser the owner marked on the
  // stats page is not counted.
  var counted = true;
  try { counted = localStorage.getItem("glance-notrack") !== "1"; } catch (e) { /* counted */ }
  function tell(event) {
    if (!counted || !navigator.sendBeacon) return;
    try { navigator.sendBeacon("/api/e", JSON.stringify(event)); } catch (e) { /* not counted */ }
  }
  // A link given out with ?from=name (a profile, a post) says where its
  // visitors come from, which an app opening it does not.
  var from = "";
  try { from = new URLSearchParams(location.search).get("from") || ""; } catch (e) { /* none */ }
  tell({ k: "view", p: location.pathname, r: document.referrer, l: lang, f: from });
  each("[data-dl=\"primary\"]", function (a) {
    a.addEventListener("click", function () {
      tell({ k: "download", p: location.pathname, s: a.href.indexOf("gitee.com") >= 0 ? "gitee" : "github", l: lang });
    });
  });

  // The header takes its glass once the page is scrolled under it.
  var top = document.querySelector(".top");
  if (top) {
    var mark = function () { top.classList.toggle("scrolled", window.scrollY > 8); };
    window.addEventListener("scroll", mark, { passive: true });
    mark();
  }

  // Each part of the page rises into place as it is scrolled to.
  if ("IntersectionObserver" in window) {
    var parts = document.querySelectorAll("section .wrap > *, .hero-text > *");
    var seen = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) { e.target.classList.add("in"); seen.unobserve(e.target); }
      });
    }, { rootMargin: "0px 0px -8% 0px" });
    Array.prototype.forEach.call(parts, function (el, i) {
      el.classList.add("rise");
      el.style.setProperty("--i", el.parentNode.classList.contains("hero-text") ? i : 0);
      seen.observe(el);
    });
  }
})();
