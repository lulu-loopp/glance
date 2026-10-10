(function () {
  "use strict";
  var $ = function (s) { return document.querySelector(s); };
  var KEY = "glance-stats-key";
  var store = {
    get: function (k) { try { return localStorage.getItem(k); } catch (e) { return null; } },
    set: function (k, v) { try { localStorage.setItem(k, v); } catch (e) { /* kept for this visit only */ } }
  };
  var key = store.get(KEY), days = 30;
  var names = (window.Intl && Intl.DisplayNames) ? new Intl.DisplayNames(["zh-CN"], { type: "region" }) : null;
  var place = function (code) { if (!code) return "未知"; try { return names ? names.of(code) : code; } catch (e) { return code; } };
  var fmt = function (n) { return (n || 0).toLocaleString("zh-CN"); };
  var pct = function (a, b) { return b ? (100 * a / b).toFixed(1) + "%" : "—"; };
  var esc = function (s) { return String(s).replace(/[&<>"]/g, function (c) { return { "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;" }[c]; }); };

  function table(el, head, rows) {
    if (!rows.length) { el.innerHTML = "<tr><td class=\"empty\">还没有数据</td></tr>"; return; }
    el.innerHTML = "<tr>" + head.map(function (h, i) { return "<th" + (i ? " class=\"num\"" : "") + ">" + h + "</th>"; }).join("") + "</tr>" +
      rows.map(function (r) { return "<tr>" + r.map(function (c, i) { return "<td" + (i ? " class=\"num\"" : "") + ">" + esc(c) + "</td>"; }).join("") + "</tr>"; }).join("");
  }

  // Every day of the range, the empty ones too.
  function everyDay(since, n, rows) {
    var by = {}; rows.forEach(function (r) { by[r.day] = r; });
    var out = [], t = Date.parse(since + "T00:00:00Z");
    for (var i = 0; i < n; i++) {
      var day = new Date(t + i * 86400000).toISOString().slice(0, 10);
      out.push(by[day] || { day: day, views: 0, visitors: 0, downloads: 0, downloaders: 0 });
    }
    return out;
  }

  function chart(rows) {
    var host = $("[data-chart]"), svg = host.querySelector("svg"), tip = $("[data-tip]");
    var W = host.clientWidth, H = 260, L = 36, R = 8, T = 8, B = 24;
    var most = Math.max(4, Math.max.apply(null, rows.map(function (r) { return r.visitors; })));
    var step = Math.pow(10, Math.floor(Math.log10(most))); var top = Math.ceil(most / step) * step;
    var x = function (i) { return L + (rows.length > 1 ? (W - L - R) * i / (rows.length - 1) : (W - L - R) / 2); };
    var y = function (v) { return T + (H - T - B) * (1 - v / top); };
    var parts = [];
    for (var g = 0; g <= 4; g++) {
      var v = top * g / 4, gy = y(v);
      parts.push("<line class=\"grid\" x1=\"" + L + "\" x2=\"" + (W - R) + "\" y1=\"" + gy + "\" y2=\"" + gy + "\"/>");
      parts.push("<text class=\"axis\" x=\"" + (L - 6) + "\" y=\"" + (gy + 4) + "\" text-anchor=\"end\">" + fmt(Math.round(v)) + "</text>");
    }
    var every = Math.ceil(rows.length / 8);
    rows.forEach(function (r, i) {
      // Every few days, and the last; one too near the last gives way to it.
      var last = i === rows.length - 1;
      if (last || (i % every === 0 && rows.length - 1 - i >= every / 2)) parts.push("<text class=\"axis\" x=\"" + x(i) + "\" y=\"" + (H - 6) + "\" text-anchor=\"middle\">" + r.day.slice(5) + "</text>");
    });
    ["visitors", "downloaders"].forEach(function (k, s) {
      var d = rows.map(function (r, i) { return (i ? "L" : "M") + x(i).toFixed(1) + "," + y(r[k]).toFixed(1); }).join("");
      parts.push("<path class=\"l" + (s + 1) + "\" d=\"" + d + "\"/>");
    });
    parts.push("<line data-cross class=\"grid\" y1=\"" + T + "\" y2=\"" + (H - B) + "\"/>");
    parts.push("<g data-dots></g>");
    svg.setAttribute("viewBox", "0 0 " + W + " " + H);
    svg.innerHTML = parts.join("");
    var cross = svg.querySelector("[data-cross]"), dots = svg.querySelector("[data-dots]");
    cross.style.display = "none";
    svg.onmousemove = function (e) {
      var box = svg.getBoundingClientRect(), px = (e.clientX - box.left) * W / box.width;
      var i = Math.max(0, Math.min(rows.length - 1, Math.round((px - L) / Math.max(1, (W - L - R)) * (rows.length - 1))));
      var r = rows[i], cx = x(i);
      cross.setAttribute("x1", cx); cross.setAttribute("x2", cx); cross.style.display = "";
      dots.innerHTML = ["visitors", "downloaders"].map(function (k, s) { return "<circle class=\"d" + (s + 1) + "\" cx=\"" + cx + "\" cy=\"" + y(r[k]) + "\" r=\"4\"/>"; }).join("");
      tip.innerHTML = "<b>" + r.day + "</b><br>独立访客 " + fmt(r.visitors) + " · 浏览 " + fmt(r.views) + "<br>点了下载的访客 " + fmt(r.downloaders) + "（" + pct(r.downloaders, r.visitors) + "）";
      tip.style.display = "block";
      var left = cx * box.width / W + 12; if (left + tip.offsetWidth > box.width) left = cx * box.width / W - tip.offsetWidth - 12;
      tip.style.left = left + "px"; tip.style.top = "8px";
    };
    svg.onmouseleave = function () { tip.style.display = "none"; cross.style.display = "none"; dots.innerHTML = ""; };
  }

  function show(data) {
    var rows = everyDay(data.since, data.days, data.daily);
    var sum = function (k) { return rows.reduce(function (a, r) { return a + (r[k] || 0); }, 0); };
    var visitors = sum("visitors"), downloaders = sum("downloaders");
    $("[data-tiles]").innerHTML = [
      ["独立访客", fmt(visitors), "按天去重后相加"],
      ["浏览", fmt(sum("views")), "打开页面的次数"],
      ["下载点击", fmt(sum("downloads")), fmt(downloaders) + " 位访客"],
      ["转化率", pct(downloaders, visitors), "点了下载的访客 / 独立访客"]
    ].map(function (t) { return "<div class=\"card tile\"><div class=\"l\">" + t[0] + "</div><div class=\"n\">" + t[1] + "</div><div class=\"s\">" + t[2] + "</div></div>"; }).join("");
    chart(rows);
    table($("[data-daily]"), ["日期", "独立访客", "浏览", "下载点击", "点了下载的访客"], rows.slice().reverse().map(function (r) { return [r.day, fmt(r.visitors), fmt(r.views), fmt(r.downloads), fmt(r.downloaders)]; }));
    table($("[data-countries]"), ["国家和地区", "访客", "下载", "转化"], data.countries.map(function (c) { return [place(c.country), fmt(c.visitors), fmt(c.downloaders), pct(c.downloaders, c.visitors)]; }));
    table($("[data-sources]"), ["来源", "下载点击"], data.sources.map(function (s) { return [s.source === "gitee" ? "Gitee" : s.source === "github" ? "GitHub" : s.source || "未知", fmt(s.downloads)]; }));
    table($("[data-pages]"), ["页面", "浏览"], data.pages.map(function (p) { return [p.path === "/" ? "中文首页" : p.path === "/en/" ? "英文首页" : p.path, fmt(p.views)]; }));
    table($("[data-refs]"), ["来源", "浏览"], data.refs.map(function (r) { return [r.ref === "" ? "不带来源（App 内打开、直接访问等）" : r.ref.indexOf("tag:") === 0 ? "标记：" + r.ref.slice(4) : r.ref, fmt(r.views)]; }));
  }

  function load() {
    fetch("/api/stats?days=" + days, { headers: { "x-stats-key": key || "" } }).then(function (r) {
      if (r.status === 403) { $("[data-board]").hidden = true; $("[data-login]").hidden = false; return null; }
      return r.json();
    }).then(function (data) {
      if (!data) return;
      // The owner's browser: kept out of the count.
      store.set(KEY, key); store.set("glance-notrack", "1");
      $("[data-login]").hidden = true; $("[data-board]").hidden = false;
      show(data);
    });
  }

  $("[data-form]").addEventListener("submit", function (e) { e.preventDefault(); key = $("[data-key]").value.trim(); load(); });
  $("[data-days]").addEventListener("click", function (e) {
    var b = e.target.closest("button[data-d]"); if (!b) return;
    days = +b.getAttribute("data-d");
    Array.prototype.forEach.call(this.querySelectorAll("button"), function (x) { x.setAttribute("aria-pressed", x === b ? "true" : "false"); });
    load();
  });
  var redraw; window.addEventListener("resize", function () { clearTimeout(redraw); redraw = setTimeout(load, 200); });
  if (key) load(); else $("[data-login]").hidden = false;
})();
