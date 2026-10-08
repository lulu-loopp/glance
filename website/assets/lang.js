// The Chinese page, opened by a browser that reads no Chinese, goes to the
// English one, before anything is drawn; unless the visitor chose a language
// with the switch at the top (kept by site.js), which is followed instead.
(function () {
  "use strict";
  var chosen = null;
  try { chosen = localStorage.getItem("glance-lang"); } catch (e) { /* storage off: the browser's languages decide */ }
  if (chosen === "zh") return;
  var languages = navigator.languages && navigator.languages.length ? navigator.languages : [navigator.language || ""];
  var reads = languages.some(function (l) { return /^zh\b/i.test(l); });
  if (chosen === "en" || !reads) location.replace("en/" + location.search + location.hash);
})();
