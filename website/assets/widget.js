// A Glance widget drawn in the page, as Glance lays it out: drag its corner
// (or its right or bottom edge) and it shows what fits the room, from the
// panel's lanes down to a few rings, each layout turning into the next.
// The layouts, the ladder that chooses among them (more shows again only
// with room to spare) and the turning are the widget's own, made up
// readings running.
(function () {
  "use strict";

  var host = document.querySelector("[data-widget]");
  if (!host) return;
  var root = document.documentElement;
  var zh = root.lang.indexOf("zh") === 0;
  var W = zh ? {
    cpu: "CPU", gpu: "GPU", mem: "内存", net: "网络", disk: "磁盘", proc: "进程", store: "存储",
    clock: "频率", power: "功耗", vram: "显存", module: "内存条", down: "下载", up: "上传", read: "读取", write: "写入",
    rd: "读", wr: "写", uptime: "已开机 3 天 22 小时", cols: ["CPU", "内存", "读写", "GPU"]
  } : {
    cpu: "CPU", gpu: "GPU", mem: "Memory", net: "Network", disk: "Disk", proc: "Processes", store: "Storage",
    clock: "Clock", power: "Power", vram: "VRAM", module: "Module", down: "Down", up: "Up", read: "Read", write: "Write",
    rd: "R", wr: "W", uptime: "Up 3 d 22 h", cols: ["CPU", "Mem", "I/O", "GPU"]
  };

  // ---------------- the window and its canvas ----------------
  var win = document.createElement("div");
  win.className = "gw-win";
  var cv = document.createElement("canvas");
  win.appendChild(cv);
  host.appendChild(win);
  var ctx = cv.getContext("2d");

  // Colours, as the look on the page has them (see site.css: .gw-win).
  var T = {};
  function readTheme() {
    var cs = getComputedStyle(win), v = function (n) { return cs.getPropertyValue(n).trim(); };
    T = { ink: v("--ink"), ink2: v("--ink-2"), ink3: v("--ink-3"), rule: v("--rule"), wash: v("--wash"), signal: v("--signal"), accent: v("--accent"), halo: v("--chip"),
      hues: { cpu: v("--h-cpu"), gpu: v("--h-gpu"), mem: v("--h-mem"), net: v("--h-net"), disk: v("--h-disk") } };
  }

  // ---------------- made-up readings ----------------
  var seed = 7;
  function rnd() { return (seed = (seed * 16807) % 2147483647) / 2147483647; }
  var N = 90, THREADS = 32;
  var H = { cpu: [], gpu: [], mem: [], down: [], up: [], read: [], write: [] };
  var R = { cpu: 38, gpu: 22, mem: 35.4, temp: 58, gtemp: 47, down: 2.4e6, up: 1.9e5, read: 4.4e7, write: 6.8e5, ghz: 5.25, watt: 92, gwatt: 51, vram: 3.0, threads: [], procs: [] };
  for (var i = 0; i < THREADS; i++) R.threads.push(rnd() * 60);
  function walk(v, lo, hi, j) { return Math.min(hi, Math.max(lo, v + (rnd() - 0.5) * j)); }
  function sample() {
    R.cpu = walk(R.cpu, 6, 72, 20); R.gpu = walk(R.gpu, 8, 78, 16); R.mem = walk(R.mem, 30, 52, 1.4);
    R.temp = 44 + R.cpu * 0.42 + (rnd() - 0.5) * 2; R.gtemp = 38 + R.gpu * 0.4; R.ghz = 4.3 + R.cpu / 100 * 1.4;
    R.watt = 40 + R.cpu * 1.6; R.gwatt = 25 + R.gpu * 2.4; R.vram = walk(R.vram, 1.5, 9, 0.4);
    R.down = Math.max(800, walk(R.down, 0, 2.6e7, 6e6)); R.up = Math.max(400, walk(R.up, 0, 2e6, 3e5));
    R.read = Math.max(0, walk(R.read, 0, 2.2e8, 4e7)); R.write = Math.max(0, walk(R.write, 0, 6e7, 8e6));
    R.threads = R.threads.map(function (t) { return walk(t * 0.6 + R.cpu * 0.4, 0, 100, 40); });
    R.procs = [["blender", 3.0], ["chrome", 1.6], ["glance", 0.9], ["dwm", 0.5], ["svchost", 0.5], ["explorer", 0.3]]
      .map(function (p) { return [p[0], Math.max(0, p[1] * (R.cpu / 30) * (0.6 + rnd() * 0.8))]; }).sort(function (a, b) { return b[1] - a[1]; });
    Object.keys(H).forEach(function (k) { H[k].push(R[k] || 0); if (H[k].length > N) H[k].shift(); });
  }
  for (var s0 = 0; s0 < N; s0++) sample();
  var HOT = 85;
  function hotNow(id) { return ({ cpu: R.cpu > HOT || R.temp > 85, gpu: R.gpu > HOT, mem: R.mem > HOT })[id] || false; }
  function rateParts(b, short) {
    var vu = b >= 1048576 ? [b / 1048576, "MB/s"] : b >= 1024 ? [b / 1024, "KB/s"] : [b, "B/s"];
    var fig = vu[0] >= 100 ? vu[0].toFixed(0) : vu[0].toFixed(1);
    return [fig, short ? (vu[1] === "B/s" ? "" : vu[1][0]) : vu[1]];
  }
  function scaleOf(s) { var m = Math.max.apply(null, s.concat([1])), p = 1024; for (;;) { for (var k of [1, 2, 5]) if (k * p >= m) return k * p; p *= 10; } }

  // ---------------- icons (Lucide, ISC: Glance's own) ----------------
  var ICONS = {
    cpu: ["M12 20v2", "M12 2v2", "M17 20v2", "M17 2v2", "M2 12h2", "M2 17h2", "M2 7h2", "M20 12h2", "M20 17h2", "M20 7h2", "M7 20v2", "M7 2v2",
      "M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z", "M9 8h6a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1z"],
    gpu: ["M2 17h18a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2H2", "M2 21V3", "M7 17v3a1 1 0 0 0 1 1h5a1 1 0 0 0 1-1v-3", "M18 11a2 2 0 1 1-4 0a2 2 0 1 1 4 0z", "M10 11a2 2 0 1 1-4 0a2 2 0 1 1 4 0z"],
    mem: ["M12 12v-2", "M12 18v-2", "M16 12v-2", "M16 18v-2", "M2 11h1.5", "M20 18v-2", "M20.5 11H22", "M4 18v-2", "M8 12v-2", "M8 18v-2", "M4 6h16a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2z"],
    disk: ["M10 16h.01", "M2.212 11.577a2 2 0 0 0-.212.896V18a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-5.527a2 2 0 0 0-.212-.896L18.55 5.11A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z", "M21.946 12.013H2.054", "M6 16h.01"],
    net: ["m3 16 4 4 4-4", "M7 20V4", "m21 8-4-4-4 4", "M17 4v16"],
    list: ["M3 5h.01", "M3 12h.01", "M3 19h.01", "M8 5h13", "M8 12h13", "M8 19h13"]
  };
  var PATHS = {};
  Object.keys(ICONS).forEach(function (k) { PATHS[k] = ICONS[k].map(function (d) { return new Path2D(d); }); });

  // ---------------- the looks ----------------
  // 记录纸: one ink, condensed figures. Windows 11: Segoe, the accent colour
  // for anything that is data. 磨砂玻璃: a hue for each reading.
  var SANS = '"Segoe UI Variable Text", "Segoe UI", -apple-system, "PingFang SC", "Microsoft YaHei UI", system-ui, sans-serif';
  var SKINS = {
    paper: { font: '"Archivo", ' + SANS, cond: true, figure: 1, tracking: 0, hue: function () { return null; } },
    fluent: { font: '"Segoe UI Variable Display", ' + SANS, cond: false, figure: 0.84, tracking: -0.02, hue: function () { return T.accent; } },
    glass: { font: SANS, cond: false, figure: 0.9, tracking: -0.035, hue: function (id) { return T.hues[id] || T.hues.cpu; }, hues: true }
  };
  var SKIN = SKINS.glass;
  function keyId(key) { return key.split(".")[0]; }

  // ---------------- measuring ----------------
  var mctx = document.createElement("canvas").getContext("2d");
  function fontOf(size, weight) { return weight + " " + size + "px " + SKIN.font; }
  function face(c, size, weight, cond) {
    c.font = fontOf(size, weight);
    try { c.fontStretch = cond && SKIN.cond ? "extra-condensed" : "normal"; } catch (e) {}
    try { c.letterSpacing = cond && !SKIN.cond ? SKIN.tracking * size + "px" : "0px"; } catch (e) {}
  }
  function figSize(size, cond) { return cond && !SKIN.cond ? size * SKIN.figure : size; }
  function measure(text, size, weight, cond) { face(mctx, figSize(size, cond), weight, cond); return mctx.measureText(text); }
  function widthOf(text, size, weight, cond) { return measure(text, size, weight, cond).width; }
  function centred(text, size, weight, middle, cond) {
    var m = measure(text, size, weight, cond);
    return middle + (m.actualBoundingBoxAscent - m.actualBoundingBoxDescent) / 2;
  }

  // ---------------- readings, as every layout shows them ----------------
  function readings() {
    var list = [];
    list.push({ id: "cpu", icon: "cpu", label: W.cpu, share: R.cpu / 100, value: R.cpu.toFixed(0), unit: "%", sub: R.temp.toFixed(0) + "°", detail: R.temp.toFixed(0) + " °C", hot: R.cpu > HOT || R.temp > 85, series: H.cpu, max: 100 });
    list.push({ id: "gpu", icon: "gpu", label: W.gpu, share: R.gpu / 100, value: R.gpu.toFixed(0), unit: "%", sub: R.gtemp.toFixed(0) + "°", detail: R.gtemp.toFixed(0) + " °C", hot: R.gpu > HOT, series: H.gpu, max: 100 });
    list.push({ id: "mem", icon: "mem", label: W.mem, share: R.mem / 100, value: R.mem.toFixed(0), unit: "%", sub: "", detail: (R.mem / 100 * 61.6).toFixed(1) + " GB", hot: R.mem > HOT, series: H.mem, max: 100 });
    function pair(id, label, marks, a, b, sa, sb) {
      return { id: id, icon: id, label: label, share: null, hot: false, series: sa, second: sb, max: scaleOf(sa.concat(sb)),
        pair: [[marks[0], a], [marks[1], b]].map(function (p) { return { mark: p[0], full: rateParts(p[1], false), short: rateParts(p[1], true) }; }) };
    }
    list.push(pair("net", W.net, ["↓", "↑"], R.down, R.up, H.down, H.up));
    list.push(pair("disk", W.disk, [W.rd, W.wr], R.read, R.write, H.read, H.write));
    return list;
  }

  // ---------------- elements ----------------
  // A layout is a list of elements, each with a key naming what it is: the
  // same key in two layouts is the same thing, moved and resized between.
  function Layout() {
    var els = [];
    var api = {
      els: els,
      text: function (key, text, x, y, size, weight, color, o) {
        o = o || {};
        if (text === "" || text == null) return 0;
        var w = widthOf(text, size, weight, o.cond);
        els.push({ t: "text", key: key, text: String(text), x: o.right ? x - w : x, y: y, size: figSize(size, o.cond), weight: weight, color: color, cond: !!o.cond, w: w });
        return w;
      },
      textMid: function (key, text, x, middle, size, weight, color, o) { return api.text(key, text, x, centred(String(text), size, weight, middle, (o || {}).cond), size, weight, color, o); },
      ring: function (key, x, y, r, icon, share, hot) { els.push({ t: "ring", key: key, x: x, y: y, r: r, icon: icon, share: share, hot: hot }); },
      icon: function (key, x, y, s, icon, color) { els.push({ t: "icon", key: key, x: x, y: y, s: s, icon: icon, color: color }); },
      chart: function (key, x, y, w, h, series, max, second, secondId, labels) { els.push({ t: "chart", key: key, x: x, y: y, w: Math.max(0, w), h: Math.max(0, h), series: series, second: second, max: max, secondId: secondId || keyId(key), labels: labels }); },
      rule: function (key, x, y, w, h) { els.push({ t: "rule", key: key, x: x, y: y, w: w, h: h }); },
      meter: function (key, x, y, w, h, frac, hot) { els.push({ t: "meter", key: key, x: x, y: y, w: w, h: h, frac: frac, hot: hot }); },
      bars: function (key, x, y, w, h) { els.push({ t: "bars", key: key, x: x, y: y, w: w, h: h }); }
    };
    return api;
  }
  function figure(L, r, x, base, size, unitSize, o) {
    o = o || {};
    var color = r.hot ? "signal" : "ink";
    if (/^[A-Za-z]/.test(r.unit)) o = Object.assign({}, o, { gap: Math.max(o.gap != null ? o.gap : 2, size * 0.25) });
    var gap = o.gap != null ? o.gap : 2;
    var fw = L.text(r.id + ".value", r.value, x, base, size, o.weight || 620, color, { cond: o.cond });
    var uw = L.text(r.id + ".unit", r.unit, x + fw + gap, base, unitSize, o.unitWeight || 500, o.unitColor || "ink2");
    return fw + gap + uw;
  }
  function rate(L, r, which, x, base, size, o) {
    o = o || {};
    var p = r.pair[which === "a" ? 0 : 1], key = r.id + "." + which, fu = o.short ? p.short : p.full;
    var mid = base - measure(fu[0], size, 600, o.cond).actualBoundingBoxAscent / 2;
    var x0 = x;
    x0 += L.textMid(key + ".mark", p.mark, x0, mid, size * 0.86, 500, "ink3") + (o.markGap != null ? o.markGap : 3);
    x0 += L.text(key + ".value", fu[0], x0, base, size, o.weight || 600, which === "a" ? "ink" : (o.second || "ink"), { cond: o.cond });
    if (fu[1]) x0 += 1 + L.text(key + ".unit", fu[1], x0 + 1, base, o.unitSize || size * 0.78, 500, "ink2");
    return x0 - x;
  }

  // ---------------- the lanes (the panel's own layout) ----------------
  var COL = 356, PADX = 18, PADT = 12, PADB = 12, HEAD = 25, PLOT = 40, RPLOT = 36, LINE = 16, FG = 3, ROW = 22, BAR = 44, LABEL = 96;
  function lanes(full) {
    function facts(rows) { return full ? rows : []; }
    return [
      { id: "cpu", title: W.cpu, dev: "AMD Ryzen 9 9950X", aside: function () { return R.temp.toFixed(0) + " °C"; }, hot: function () { return R.temp > 85; },
        blocks: [["readout", "cpu"], ["facts", facts([[W.clock, function () { return R.ghz.toFixed(2) + " GHz"; }], [W.power, function () { return R.watt.toFixed(1) + " W"; }], ["CCD 1", function () { return (R.temp + 1).toFixed(0) + " °C"; }], ["CCD 2", function () { return (R.temp - 18).toFixed(0) + " °C"; }]])]].concat(full ? [["threads"]] : []) },
      { id: "gpu", title: W.gpu, dev: "NVIDIA GeForce RTX 5070 Ti", aside: function () { return R.gtemp.toFixed(0) + " °C"; }, hot: function () { return false; },
        blocks: [["readout", "gpu"], ["meter", W.vram, function () { return R.vram / 16; }, function () { return R.vram.toFixed(1) + " / 15.6 GB"; }], ["facts", facts([[W.clock, function () { return (1500 + R.gpu * 12).toFixed(0) + " MHz"; }], [W.power, function () { return R.gwatt.toFixed(1) + " W"; }]])]] },
      { id: "mem", title: W.mem, dev: "2 × 32 GB DDR5-6000", aside: function () { return (R.mem / 100 * 61.6).toFixed(1) + " / 61.6 GB"; }, hot: function () { return false; },
        blocks: [["readout", "mem"], ["facts", facts([[W.module + " 1", function () { return "37 °C"; }], [W.module + " 2", function () { return "36 °C"; }]])]] },
      { id: "net", title: W.net, dev: "Intel Wi-Fi 6E AX210", aside: function () { return ""; }, hot: function () { return false; }, blocks: [["rates", "net", [W.down, W.up]]] },
      { id: "disk", title: W.disk, dev: "Sandisk Optimus 5100 2TB", aside: function () { return "34 °C"; }, hot: function () { return false; }, blocks: [["rates", "disk", [W.read, W.write]]] },
      { id: "proc", title: W.proc, dev: "", aside: function () { return ""; }, hot: function () { return false; }, blocks: [["table", full ? 5 : 3]] },
      { id: "store", title: W.store, dev: "", aside: function () { return ""; }, hot: function () { return false; }, blocks: [["meter0", "C:", function () { return 0.47; }, function () { return "283 / 600 GB"; }], ["meter", "D:", function () { return 0.39; }, function () { return "496 / 1262 GB"; }]] }
    ];
  }
  function blockHeight(b) {
    switch (b[0]) {
      case "readout": return PLOT; case "rates": return RPLOT; case "threads": return 24; case "meter": return 8 + LINE; case "meter0": return LINE;
      case "facts": return b[1].length ? 10 + b[1].length * LINE + (b[1].length - 1) * FG : 0; case "table": return b[1] * ROW;
    }
    return 0;
  }
  function laneHeight(l) { return PADT + HEAD + l.blocks.reduce(function (a, b) { return a + blockHeight(b); }, 0) + PADB; }
  function charted(l) { return l.blocks.filter(function (b) { return b[0] === "readout" || b[0] === "rates"; }).length; }
  function split(hs, n) {
    n = Math.min(n, hs.length);
    var best = null;
    (function go(start, left, cuts) {
      if (left === 1) {
        var all = cuts.concat([hs.length]), prev = 0, tall = 0;
        all.forEach(function (c) { tall = Math.max(tall, hs.slice(prev, c).reduce(function (a, b) { return a + b; }, 0)); prev = c; });
        if (!best || tall < best.tall) best = { cuts: all, tall: tall };
        return;
      }
      for (var c = start + 1; c <= hs.length - left + 1; c++) go(c, left - 1, cuts.concat([c]));
    })(0, n, []);
    return best;
  }
  function lanesLayout(full, cols) {
    var list = lanes(full), s = split(list.map(laneHeight), cols), columns = [], prev = 0;
    s.cuts.forEach(function (c) { columns.push(list.slice(prev, c)); prev = c; });
    return { columns: columns, w: columns.length * COL, h: s.tall + BAR };
  }
  function layLane(L, lane, x, y, w, grow) {
    var rs = {};
    readings().forEach(function (r) { rs[r.id] = r; });
    var left = x + PADX, width = w - 2 * PADX, plotL = left + LABEL + 12, plotW = left + width - plotL, id = lane.id;
    var yy = y + PADT, row = yy + 13;
    var tw = L.text(id + ".label", lane.title, left, row, 13, 650, "ink");
    var aside = lane.aside(), aw = aside ? widthOf(aside, 13, 500) + 8 : 0;
    L.text(id + ".detail", aside, left + width, row, 13, 500, lane.hot() ? "signal" : "ink2", { right: true });
    var dev = lane.dev, room = width - tw - 8 - aw;
    while (dev && widthOf(dev, 12, 500) > room) dev = dev.slice(0, -2) + "…";
    L.text(id + ".device", dev, left + tw + 8, row, 12, 500, "ink3");
    yy += HEAD;
    var per = Math.min(RPLOT, grow / Math.max(1, charted(lane)));
    lane.blocks.forEach(function (b) {
      var bh = blockHeight(b) + (b[0] === "readout" || b[0] === "rates" ? per : 0), r;
      if (b[0] === "readout") { r = rs[b[1]]; figure(L, r, left, yy + bh - 4, 42, 15, { cond: true, gap: 3 }); L.chart(id + ".chart", plotL, yy, plotW, bh, r.series, r.max); }
      if (b[0] === "rates") {
        r = rs[b[1]];
        ["a", "b"].forEach(function (which, i) {
          var ry = yy + bh - (2 - i) * (LINE + 2) + 1 + 12, p = r.pair[i];
          L.rule(null, left, ry - 6 - (i ? 0 : 0.5), 10, i ? 1 : 2);
          L.text(id + "." + which + ".mark", b[2][i], left + 15, ry, 12, 500, "ink2");
          var uw = widthOf(p.full[1], 12, 500);
          L.text(id + "." + which + ".unit", p.full[1], left + LABEL + 26, ry, 12, 500, "ink2", { right: true });
          L.text(id + "." + which + ".value", p.full[0], left + LABEL + 26 - uw - 3, ry, 13, 500, "ink", { right: true });
        });
        L.chart(id + ".chart", plotL + 26, yy, plotW - 26, bh, r.series, r.max, r.second);
      }
      if (b[0] === "facts") b[1].forEach(function (f, i) { var ry = yy + 10 + i * (LINE + FG) + 12; L.text(id + ".f" + i + ".l", f[0], left, ry, 12, 500, "ink2"); L.text(id + ".f" + i + ".v", f[1](), plotL, ry, 12, 500, "ink"); });
      if (b[0] === "threads") L.bars("cpu.threads", left, yy + 10, width, 14);
      if (b[0] === "meter" || b[0] === "meter0") {
        var my = yy + (b[0] === "meter" ? 8 : 0);
        L.text(id + "." + b[1] + ".l", b[1], left, my + 12, 12, 500, "ink2");
        var vw = L.text(id + "." + b[1] + ".v", b[3](), left + width, my + 12, 12, 500, "ink2", { right: true });
        L.meter(id + "." + b[1] + ".m", plotL, my + 6.5, left + width - vw - 12 - plotL, 3, b[2]());
      }
      if (b[0] === "table") {
        var cols = [[W.cols[0], 40], [W.cols[1], 52], [W.cols[2], 62], [W.cols[3], 36]], cx = left + width, xs = [];
        for (var k = cols.length - 1; k >= 0; k--) { xs[k] = cx; cx -= cols[k][1] + 6; }
        cols.forEach(function (c, k) { L.text("proc.h" + k, c[0], xs[k], yy - HEAD + 13, 12, k ? 500 : 650, k ? "ink2" : "ink", { right: true }); });
        R.procs.slice(0, b[1]).forEach(function (p, k) {
          var ry = yy + k * ROW + 15;
          L.text("proc." + k + ".n", p[0], left, ry, 13, 500, "ink");
          [p[1].toFixed(1) + "%", (80 + k * 97) + " MB", (1.2 + k * 0.7).toFixed(1) + " MB/s", (k ? 0.1 * k : 2.1).toFixed(1) + "%"].forEach(function (v, j) { L.text("proc." + k + "." + j, v, xs[j], ry, 12, 500, "ink2", { right: true }); });
        });
      }
      yy += bh;
    });
  }
  function layLanes(L, full, cols, w, h, ex) {
    var l = lanesLayout(full, cols), cw = COL + ex.w / l.columns.length;
    l.columns.forEach(function (col, ci) {
      var y = 0, x = ci * cw, per = ex.h / col.length;
      col.forEach(function (lane, li) { if (li) L.rule(null, x + PADX, y, cw - 2 * PADX, 1); layLane(L, lane, x, y, cw, per); y += laneHeight(lane) + per; });
      if (ci) L.rule(null, x, 0, 1, h - BAR);
    });
    L.rule(null, 0, h - BAR, w, 1);
    L.text("bar.uptime", W.uptime, PADX, h - 17, 12, 500, "ink2");
    L.icon("bar.list", w - 30, h - 22, 16, "list", "ink2");
  }

  // ---------------- the small layouts ----------------
  var TILE = { w: 156, h: 112 };
  function keys(ids, list) { return ids.map(function (id) { return list.find(function (r) { return r.id === id; }); }).filter(Boolean); }
  function layTiles(L, c, rows, w, h) {
    var list = readings().slice(0, c * rows), tw = (w - 24) / c, th = (h - 24) / rows;
    list.forEach(function (r, i) {
      var x = 12 + (i % c) * tw, y = 12 + Math.floor(i / c) * th, below;
      if (i % c) L.rule(null, x, y + 8, 1, th - 16);
      if (i >= c) L.rule(null, x + 8, y, tw - 16, 1);
      L.ring(r.id + ".icon", x + 22, y + 22, 11, r.icon, r.share, r.hot);
      L.textMid(r.id + ".label", r.label, x + 40, y + 22, 12, 500, "ink2");
      if (r.detail) L.textMid(r.id + ".detail", r.detail, x + tw - 12, y + 22, 12, 500, "ink3", { right: true });
      if (r.pair) {
        rate(L, r, "a", x + 12, y + 58, 17, { cond: true, weight: 620, unitSize: 11 });
        rate(L, r, "b", x + 12, y + 78, 17, { cond: true, weight: 620, unitSize: 11 });
        below = y + 86;
      } else {
        figure(L, r, x + 12, y + 70, 30, 12, { cond: true, gap: 3 });
        below = y + 78;
      }
      L.chart(r.id + ".chart", x + 12, below, tw - 24, Math.max(14, y + th - 10 - below), r.series, r.max, r.second);
    });
  }
  function chip(L, r, x, middle, size, sub, ringR) {
    ringR = ringR || 9.5;
    L.ring(r.id + ".icon", x + ringR, middle, ringR, r.icon, r.share, r.hot);
    var x0 = x + ringR * 2 + 6;
    if (r.pair) {
      x0 += rate(L, r, "a", x0, centred("8", size, 600, middle), size, { short: true }) + 6;
      x0 += rate(L, r, "b", x0, centred("8", size, 600, middle), size, { short: true, second: "ink2" });
      return x0 - x;
    }
    var base = centred("8", size, 600, middle);
    x0 += figure(L, r, x0, base, size, size, { weight: 600, gap: 0.5, unitWeight: 600, unitColor: r.hot ? "signal" : "ink" });
    if (sub && r.sub) x0 += 5 + L.text(r.id + ".sub", r.sub, x0 + 5, base, size, 500, "ink3");
    return x0 - x;
  }
  function chipWidth(r, size, sub, ringR) { return chip(Layout(), r, 0, 20, size, sub, ringR); }
  function row(L, list, left, right, middle, size, sub, ringR) {
    var ws = list.map(function (r) { return chipWidth(r, size, sub, ringR); });
    var gap = list.length > 1 ? (right - left - ws.reduce(function (a, b) { return a + b; }, 0)) / (list.length - 1) : 0, x = left;
    list.forEach(function (r, i) { chip(L, r, x, middle, size, sub, ringR); x += ws[i] + gap; });
  }
  var BOTH = ["CPU", "GPU"];
  function layCorner(L, w, h) {
    var list = readings(), pad = 14;
    L.chart("cpu.chart", pad, pad, w - 2 * pad, h - 2 * pad - 70, H.cpu, 100, H.gpu, "gpu", BOTH);
    row(L, keys(["cpu", "gpu", "mem"], list), pad, w - pad, h - pad - 46, 14, true, 10);
    row(L, keys(["net", "disk"], list), pad, w - pad, h - pad - 12, 13, false, 10);
  }
  function layStrip(L, rich, w, h) {
    var list = keys(["cpu", "gpu", "mem", "net"], readings()), parts = [], mid = h / 2;
    if (rich) parts.push({ w: 84, lay: function (x) { L.chart("cpu.chart", x, 7, 84, h - 14, H.cpu, 100); } });
    list.forEach(function (r) { parts.push({ w: chipWidth(r, 14, rich), lay: function (x) { chip(L, r, x, mid, 14, rich, 8.5); } }); });
    var used = parts.reduce(function (a, p) { return a + p.w; }, 0), gap = Math.max(10, (w - 24 - used) / (parts.length - 1)), x = 12;
    parts.forEach(function (p) { p.lay(x); x += p.w + gap; });
  }
  function layRail(L, w, h) {
    var list = keys(["cpu", "gpu", "mem", "net", "disk"], readings()), pad = 10, each = 36;
    var ch = Math.max(36, h - 2 * pad - list.length * each - 8);
    L.chart("cpu.chart", pad, pad, w - 2 * pad, ch, H.cpu, 100, H.gpu, "gpu", BOTH);
    var gap = Math.max(0, (h - 2 * pad - ch - 8 - list.length * each) / list.length), wide = w >= 150;
    list.forEach(function (r, i) {
      var top = pad + ch + 8 + i * (each + gap) + gap / 2, mid = top + 11;
      L.ring(r.id + ".icon", pad + 9, mid, 8.5, r.icon, r.share, r.hot);
      var x = pad + 24, base = centred("8", 13, 600, mid), base2 = centred("8", 11, 500, mid + 15);
      if (r.pair) {
        var aw = rate(L, r, "a", x, base, 13, { short: true });
        if (wide) rate(L, r, "b", x + aw + 6, base, 13, { short: true, second: "ink2" });
        else rate(L, r, "b", x, base2, 11, { short: true, second: "ink2" });
        return;
      }
      var fw = figure(L, r, x, base, 13, 13, { weight: 600, gap: 0.5, unitWeight: 600, unitColor: r.hot ? "signal" : "ink" });
      if (r.sub) { if (wide) L.text(r.id + ".sub", r.sub, x + fw + 5, base, 13, 500, "ink3"); else L.text(r.id + ".sub", r.sub, x, base2, 11, 500, "ink3"); }
    });
  }
  function layMicro(L, w, h) {
    var list = keys(["cpu", "gpu", "mem"], readings()), cell = (w - 16) / list.length;
    list.forEach(function (r, i) { L.ring(r.id + ".icon", 8 + cell * (i + 0.5), h / 2, Math.min(h / 2 - 5, 13), r.icon, r.share, r.hot); });
  }

  // ---------------- the ladder ----------------
  var LADDER = [];
  [true, false].forEach(function (full) {
    [3, 2, 1].forEach(function (cols) {
      LADDER.push({ key: (full ? "full" : "compact") + cols, level: full ? 0 : 1, floor: 1,
        measure: function () { var l = lanesLayout(full, cols); return { w: l.w, h: l.h }; }, stretch: { w: 120 * cols, h: 60 },
        lay: function (L, w, h, ex) { layLanes(L, full, cols, w, h, ex); } });
    });
  });
  function tileRows(c) { return Math.ceil(readings().length / c); }
  [3, 2].forEach(function (c) {
    LADDER.push({ key: "tiles" + c, level: 2, floor: 0.9, measure: function () { return { w: c * TILE.w + 24, h: tileRows(c) * TILE.h + 24 }; },
      stretch: { w: 36 * c, h: 30 * tileRows(c) }, lay: function (L, w, h) { layTiles(L, c, tileRows(c), w, h); } });
  });
  LADDER.push({ key: "corner", level: 3, floor: 0.85, measure: function () { return { w: 300, h: 176 }; }, stretch: { w: 200, h: 170 }, lay: layCorner });
  LADDER.push({ key: "strip", level: 4, floor: 0.76, measure: function () { return { w: 700, h: 36 }; }, stretch: { w: 260, h: 10 }, lay: function (L, w, h) { layStrip(L, true, w, h); } });
  LADDER.push({ key: "rail", level: 4, floor: 0.76, measure: function () { return { w: 96, h: 120 + readings().length * 36 }; }, stretch: { w: 96, h: 160 }, lay: layRail });
  LADDER.push({ key: "strip2", level: 5, floor: 0.76, measure: function () { return { w: 420, h: 36 }; }, stretch: { w: 120, h: 10 }, lay: function (L, w, h) { layStrip(L, false, w, h); } });
  LADDER.push({ key: "micro", level: 6, floor: 0.9, measure: function () { return { w: 120, h: 40 }; }, stretch: { w: 30, h: 6 }, lay: layMicro });

  var MAX_SCALE = 1.3, STICK = 1.05, UP = 1.08;
  function fit(c, Wd, Ht) {
    var m = c.measure(), s = Math.min(Wd / m.w, Ht / m.h);
    if (s < c.floor) return null;
    var scale = Math.min(s, MAX_SCALE);
    var ex = { w: Math.min(c.stretch.w, Math.max(0, Wd / scale - m.w)), h: Math.min(c.stretch.h, Math.max(0, Ht / scale - m.h)) };
    return { c: c, scale: scale, ex: ex, w: (m.w + ex.w) * scale, h: (m.h + ex.h) * scale };
  }
  // The level showing the most of those that fit; within it, the layout
  // filling the room best, the one shown kept unless another fills it
  // clearly better. More shows again only once the room is UP times as wide
  // or tall as when what shows was taken.
  function chooseFor(Wd, Ht, shown, taken) {
    var grown = !taken || Wd >= taken.w * UP || Ht >= taken.h * UP;
    var area = Wd * Ht, all = LADDER.map(function (c) { return fit(c, Wd, Ht); }).filter(Boolean)
      .filter(function (f) { return grown || !shown || f.c.level >= shown.level; });
    if (!all.length) { var c = LADDER[LADDER.length - 1], m = c.measure(); return { c: c, scale: c.floor, ex: { w: 0, h: 0 }, w: m.w * c.floor, h: m.h * c.floor }; }
    var level = Math.min.apply(null, all.map(function (f) { return f.c.level; }));
    var mine = all.filter(function (f) { return f.c.level === level; });
    function cover(f) { return f.w * f.h / area; }
    var best = mine.reduce(function (a, f) { return cover(f) > cover(a) ? f : a; });
    var kept = shown && mine.find(function (f) { return f.c === shown; });
    return kept && cover(best) < cover(kept) * STICK ? kept : best;
  }
  function elementsOf(f) {
    var m = f.c.measure(), L = Layout();
    f.c.lay(L, m.w + f.ex.w, m.h + f.ex.h, f.ex);
    var s = f.scale;
    return L.els.map(function (e) {
      var o = Object.assign({}, e);
      ["x", "y", "w", "h", "r", "s", "size"].forEach(function (k) { if (o[k] != null) o[k] *= s; });
      return o;
    });
  }

  // ---------------- turning from one layout into the next ----------------
  function lerp(a, b, t) { return a + (b - a) * t; }
  function morph(from, to, t) {
    var pending = new Map();
    from.forEach(function (e) { if (e.key) pending.set(e.key, e); });
    var out = [];
    to.forEach(function (b) {
      var a = b.key ? pending.get(b.key) : null;
      if (!a || a.t !== b.t) { out.push(Object.assign({}, b, { alpha: (b.alpha == null ? 1 : b.alpha) * t })); return; }
      pending.delete(b.key);
      var m = Object.assign({}, b);
      ["x", "y", "w", "h", "r", "s", "size"].forEach(function (k) { if (a[k] != null && b[k] != null) m[k] = lerp(a[k], b[k], t); });
      if (b.t === "text" && (a.text !== b.text || a.cond !== b.cond || a.weight !== b.weight || a.color !== b.color)) {
        out.push(Object.assign({}, a, { x: m.x, y: m.y, size: m.size, alpha: (a.alpha == null ? 1 : a.alpha) * (1 - t) }));
        out.push(Object.assign({}, m, { alpha: (b.alpha == null ? 1 : b.alpha) * t }));
      } else out.push(m);
    });
    from.forEach(function (a) { if (!a.key || pending.get(a.key) === a) out.push(Object.assign({}, a, { alpha: (a.alpha == null ? 1 : a.alpha) * (1 - t) })); });
    return out;
  }
  function smooth(t) { return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2; }

  // ---------------- drawing ----------------
  function colorOf(c) { return ({ ink: T.ink, ink2: T.ink2, ink3: T.ink3, signal: T.signal })[c] || c; }
  function drawIcon(c, kind, cx, cy, s, color) {
    c.save(); c.translate(cx - s / 2, cy - s / 2); c.scale(s / 24, s / 24);
    c.strokeStyle = color; c.lineWidth = 2; c.lineCap = "round"; c.lineJoin = "round";
    PATHS[kind].forEach(function (p) { c.stroke(p); });
    c.restore();
  }
  function withAlpha(c, a) {
    if (c.charAt(0) === "#") { var n = parseInt(c.slice(1), 16); return "rgba(" + (n >> 16) + "," + ((n >> 8) & 255) + "," + (n & 255) + "," + a + ")"; }
    var m = c.match(/[\d.]+/g); return "rgba(" + m[0] + "," + m[1] + "," + m[2] + "," + ((m[3] == null ? 1 : +m[3]) * a) + ")";
  }
  function drawChart(c, e) {
    var x = e.x, y = e.y, w = e.w, h = e.h;
    if (w < 2 || h < 2) return;
    var hue = SKIN.hue(keyId(e.key)), hue2 = SKIN.hue(e.secondId);
    c.fillStyle = T.rule;
    for (var i = 0; i <= 4; i++) c.fillRect(Math.round(x + w * i / 4), y, 1, h);
    function line(s, fill, color, width, dash) {
      if (!s || s.length < 2) return;
      var pts = s.map(function (v, k) { return [x + w * k / (s.length - 1), y + h - h * Math.min(1, v / e.max)]; });
      c.beginPath(); pts.forEach(function (p, k) { if (k) c.lineTo(p[0], p[1]); else c.moveTo(p[0], p[1]); });
      if (fill) { c.lineTo(x + w, y + h); c.lineTo(x, y + h); c.closePath(); c.fillStyle = hue ? withAlpha(hue, 0.16) : T.wash; c.fill(); return; }
      c.setLineDash(dash || []); c.strokeStyle = color; c.lineWidth = width; c.stroke(); c.setLineDash([]);
    }
    var own = e.secondId === keyId(e.key), first = keyId(e.key);
    var ink1 = hotNow(first) ? T.signal : hue || T.ink;
    var ink2 = !own && hotNow(e.secondId) ? T.signal : hue2 && (own || SKIN.hues) ? withAlpha(hue2, own ? 0.6 : 0.9) : T.ink3;
    line(e.series, true); line(e.series, false, ink1, 1.5);
    if (e.second) line(e.second, false, ink2, 1.2, [2, 2]);
    if (e.labels && e.second && !SKIN.hues && h >= 40) {
      // Which line is which, in the chart's top left corner.
      var size = 10.5, pad = 5, sample = 12, gap = 4, apart = 9;
      face(c, size, 500, false);
      var widths = e.labels.map(function (t) { return c.measureText(t).width; });
      var lw = widths.reduce(function (a, b) { return a + b; }, 0) + 2 * (sample + gap) + apart + 2 * pad;
      var bx = x + 4, by = y + 4, bh = size + 2 * pad - 2;
      if (lw > w - 8) return;
      c.fillStyle = T.halo; c.beginPath(); c.roundRect(bx, by, lw, bh, bh / 2); c.fill();
      var lx = bx + pad, mid = by + bh / 2;
      [[ink1, [], 1.5], [ink2, [2, 2], 1.2]].forEach(function (d, k) {
        c.setLineDash(d[1]); c.strokeStyle = d[0]; c.lineWidth = d[2]; c.beginPath(); c.moveTo(lx, mid); c.lineTo(lx + sample, mid); c.stroke(); c.setLineDash([]);
        lx += sample + gap;
        c.fillStyle = T.ink2; c.fillText(e.labels[k], lx, centred(e.labels[k], size, 500, mid));
        lx += widths[k] + apart;
      });
    }
  }
  function drawElements(c, els) {
    els.forEach(function (e) {
      var a = e.alpha == null ? 1 : e.alpha;
      if (a <= 0.003) return;
      c.globalAlpha = a;
      switch (e.t) {
        case "text":
          face(c, e.size, e.weight, e.cond);
          c.fillStyle = colorOf(e.color); c.textAlign = "left"; c.textBaseline = "alphabetic"; c.fillText(e.text, e.x, e.y); break;
        case "ring": {
          var col = e.hot ? T.signal : SKIN.hue(keyId(e.key)) || T.ink; c.lineCap = "round";
          if (e.share != null) {
            c.strokeStyle = T.wash; c.lineWidth = Math.max(1.5, e.r * 0.16); c.beginPath(); c.arc(e.x, e.y, e.r, 0, 7); c.stroke();
            if (e.share > 0.005) { c.strokeStyle = col; c.beginPath(); c.arc(e.x, e.y, e.r, -Math.PI / 2, -Math.PI / 2 + Math.PI * 2 * Math.min(1, e.share)); c.stroke(); }
          } else { c.fillStyle = T.wash; c.beginPath(); c.arc(e.x, e.y, e.r * 1.1, 0, 7); c.fill(); }
          drawIcon(c, e.icon, e.x, e.y, e.r * 1.3, col); break;
        }
        case "icon": drawIcon(c, e.icon, e.x, e.y, e.s, colorOf(e.color)); break;
        case "chart": drawChart(c, e); break;
        case "rule": c.fillStyle = T.rule; c.fillRect(e.x, e.y, e.w, e.h); break;
        case "meter": c.fillStyle = T.wash; c.fillRect(e.x, e.y, e.w, e.h); c.fillStyle = e.hot ? T.signal : SKIN.hue(keyId(e.key)) || T.ink; c.fillRect(e.x, e.y, e.w * e.frac, e.h); break;
        case "bars": {
          var n = R.threads.length, g = 2, bw = (e.w - g * (n - 1)) / n;
          R.threads.forEach(function (v, k) {
            var bx = e.x + k * (bw + g); c.fillStyle = T.wash; c.fillRect(bx, e.y, bw, e.h);
            var fh = e.h * v / 100; c.fillStyle = v > HOT ? T.signal : SKIN.hue("cpu") || T.ink; c.fillRect(bx, e.y + e.h - fh, bw, fh);
          });
          break;
        }
      }
    });
    c.globalAlpha = 1;
  }

  // ---------------- the widget: moved and sized by hand ----------------
  // As Glance's: pressed within EDGE of an edge (CORNER of a corner) it is
  // sized, else moved. Sized, its edges follow the hand only GIVE of the way
  // past what its layout fills, and let go, it settles onto that.
  var MORPH = 320, EDGE = 8, CORNER = 20, GIVE = 0.5, SETTLE = 260, MARGIN = 16;
  var at = { x: MARGIN, y: MARGIN }, roomNow = null, surface = null, settle = null;
  var shown = null, taken = null, turning = null, last = [], drag = null, keeps = { l: false, t: false };
  var wanted = { w: 520, h: 400 };
  function desk() { var r = host.getBoundingClientRect(); return { w: r.width, h: r.height }; }
  function ease(t) { return 1 - Math.pow(1 - Math.min(1, Math.max(0, t)), 4); }
  function give(have, fill) { return have <= fill ? fill : fill + (have - fill) * GIVE; }
  function choose(room, now) {
    var next = chooseFor(room.w, room.h, shown && shown.c, taken && shown && taken.c === shown.c ? taken : null);
    if (!shown || next.c !== shown.c) {
      if (shown) turning = { from: last, at: now };
      taken = { w: room.w, h: room.h, c: next.c };
    }
    shown = next;
  }
  // Kept on the desk: where it is, as far as its size lets it.
  function keepIn() {
    var d = desk();
    at.x = Math.max(0, Math.min(at.x, d.w - surface.w));
    at.y = Math.max(0, Math.min(at.y, d.h - surface.h));
  }
  function paint(now) {
    // Laid out for what it shows (stretched as far as that stretches).
    var f = shown, fill = { w: f.w, h: f.h };
    var size;
    if (roomNow) size = { w: give(roomNow.w, fill.w), h: give(roomNow.h, fill.h) };
    else if (settle) {
      var t = (now - settle.at) / SETTLE, e = ease(t);
      size = { w: settle.from.w + (fill.w - settle.from.w) * e, h: settle.from.h + (fill.h - settle.from.h) * e };
      if (t >= 1) settle = null;
    } else size = fill;
    // Sized by its left or top edge, it grows and shrinks from the other.
    if (surface && keeps.l) at.x += surface.w - size.w;
    if (surface && keeps.t) at.y += surface.h - size.h;
    surface = size;
    if (!drag) keepIn();
    var k = window.devicePixelRatio || 1, w = size.w, h = size.h;
    win.style.left = at.x + "px";
    win.style.top = at.y + "px";
    win.style.width = w + "px";
    win.style.height = h + "px";
    if (cv.width !== Math.round(w * k) || cv.height !== Math.round(h * k)) { cv.width = Math.round(w * k); cv.height = Math.round(h * k); }
    var els = elementsOf(f);
    if (turning) {
      var s = (now - turning.at) / MORPH;
      if (s >= 1) turning = null; else els = morph(turning.from, els, smooth(s));
    }
    last = els;
    // What it shows keeps to the edges not dragged.
    var dx = keeps.l ? w - f.w : 0, dy = keeps.t ? h - f.h : 0;
    ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.clearRect(0, 0, cv.width, cv.height); ctx.setTransform(k, 0, 0, k, k * dx, k * dy);
    drawElements(ctx, els);
  }
  var queued = false;
  function frame(now) {
    queued = false;
    paint(now);
    if (turning || drag || settle) request();
  }
  function request() { if (!queued) { queued = true; requestAnimationFrame(frame); } }

  // Pressed on its rectangle, as on Glance's window: a rounded corner's
  // outside takes the corner too (the page itself hit-tests the curve).
  function within(e) {
    var r = win.getBoundingClientRect();
    return e.clientX >= r.left && e.clientX < r.right && e.clientY >= r.top && e.clientY < r.bottom;
  }
  function edgesAt(e) {
    var r = win.getBoundingClientRect(), x = e.clientX - r.left, y = e.clientY - r.top, w = r.width, h = r.height;
    var l = x < EDGE, rr = x > w - EDGE, t = y < EDGE, b = y > h - EDGE, side = l || rr, end = t || b;
    var edges = { l: l || end && x < CORNER, r: rr || end && x > w - CORNER, t: t || side && y < CORNER, b: b || side && y > h - CORNER };
    return edges.l || edges.r || edges.t || edges.b ? edges : null;
  }
  function cursorFor(edges) {
    if (!edges) return "grab";
    if (edges.l && edges.t || edges.r && edges.b) return "nwse-resize";
    if (edges.r && edges.t || edges.l && edges.b) return "nesw-resize";
    return edges.l || edges.r ? "ew-resize" : "ns-resize";
  }
  host.addEventListener("pointerdown", function (e) {
    if (e.button !== 0 || !within(e)) return;
    var edges = edgesAt(e);
    drag = { x: e.clientX, y: e.clientY, at: { x: at.x, y: at.y }, edges: edges, room: { w: surface.w, h: surface.h } };
    settle = null;
    if (edges) { keeps = { l: edges.l, t: edges.t }; roomNow = { w: surface.w, h: surface.h }; }
    host.setPointerCapture(e.pointerId);
    host.classList.add("handled");
    host.style.cursor = edges ? cursorFor(edges) : "grabbing";
    e.preventDefault();
    request();
  });
  host.addEventListener("pointermove", function (e) {
    if (!drag) { host.style.cursor = within(e) ? cursorFor(edgesAt(e)) : ""; return; }
    var dx = e.clientX - drag.x, dy = e.clientY - drag.y, d = desk(), ed = drag.edges;
    if (!ed) {
      at.x = Math.max(0, Math.min(drag.at.x + dx, d.w - surface.w));
      at.y = Math.max(0, Math.min(drag.at.y + dy, d.h - surface.h));
      request();
      return;
    }
    // The room the hand gives, within the desk.
    var w = drag.room.w + (ed.r ? dx : ed.l ? -dx : 0), h = drag.room.h + (ed.b ? dy : ed.t ? -dy : 0);
    var right = drag.at.x + drag.room.w, bottom = drag.at.y + drag.room.h;
    w = Math.max(1, Math.min(w, ed.l ? right : d.w - drag.at.x));
    h = Math.max(1, Math.min(h, ed.t ? bottom : d.h - drag.at.y));
    roomNow = { w: w, h: h };
    wanted = roomNow;
    choose(roomNow, performance.now());
  });
  function letGo() {
    if (!drag) return;
    if (roomNow) { roomNow = null; settle = { at: performance.now(), from: { w: surface.w, h: surface.h } }; }
    drag = null;
    host.classList.remove("handled");
    host.style.cursor = "";
    request();
  }
  host.addEventListener("pointerup", letGo);
  host.addEventListener("pointercancel", letGo);

  // The look: as the page's buttons have it.
  function look() {
    var name = root.getAttribute("data-look");
    SKIN = SKINS[name === "win11" ? "fluent" : name === "paper" ? "paper" : "glass"];
    readTheme();
    request();
  }
  new MutationObserver(look).observe(root, { attributes: true, attributeFilter: ["data-look"] });

  setInterval(function () {
    if (document.hidden || host.hidden) return;
    sample();
    request();
  }, 1000);

  // ---- panel or widget: one switch, the looks for both ----
  var seg = document.querySelector("[data-shape]");
  var stage = document.querySelector("[data-demo]");
  function shape(name) {
    var widget = name === "widget";
    if (stage) stage.hidden = widget;
    host.hidden = !widget;
    if (seg) Array.prototype.forEach.call(seg.querySelectorAll("button"), function (b) {
      var on = b.getAttribute("data-shape") === name;
      b.setAttribute("aria-pressed", on ? "true" : "false");
      if (on) {
        seg.style.setProperty("--x", b.offsetLeft + "px");
        seg.style.setProperty("--w", b.offsetWidth + "px");
      }
    });
    if (widget) {
      // Laid out for the room last asked of it, as much of it as the desk has.
      look();
      var d = desk();
      choose({ w: Math.min(wanted.w, d.w - 2 * MARGIN), h: Math.min(wanted.h, d.h - 2 * MARGIN) }, performance.now());
      request();
    }
  }
  if (seg) {
    seg.addEventListener("click", function (e) {
      var b = e.target.closest("button[data-shape]");
      if (b) shape(b.getAttribute("data-shape"));
    });
    window.addEventListener("resize", function () { shape(host.hidden ? "panel" : "widget"); });
  }
  shape("panel");
})();
