// A Glance widget drawn in the page, on the demo's readings (see demo.js):
// drag its corner and it lays itself out for the room, as the widget on the
// desktop does: rings, a strip, a rail, a chart card, tiles, the lanes.
(function () {
  "use strict";

  var demo = window.GlanceDemo;
  var host = document.querySelector("[data-widget]");
  if (!demo || !host) return;
  var R = demo.readings, el = demo.el, svg = demo.svg, chart = demo.chart, T = demo.words;
  var zh = document.documentElement.lang.indexOf("zh") === 0;

  // The readings it shows, each with its icon (Glance's own, from Lucide)
  // and its colour.
  var ICONS = {
    cpu: ["M12 20v2", "M12 2v2", "M17 20v2", "M17 2v2", "M2 12h2", "M2 17h2", "M2 7h2", "M20 12h2", "M20 17h2", "M20 7h2", "M7 20v2", "M7 2v2",
      "M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z", "M9 8h6a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1z"],
    gpu: ["M2 17h18a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2H2", "M2 21V3", "M7 17v3a1 1 0 0 0 1 1h5a1 1 0 0 0 1-1v-3",
      "M18 11a2 2 0 1 1-4 0a2 2 0 1 1 4 0z", "M10 11a2 2 0 1 1-4 0a2 2 0 1 1 4 0z"],
    mem: ["M12 12v-2", "M12 18v-2", "M16 12v-2", "M16 18v-2", "M2 11h1.5", "M20 18v-2", "M20.5 11H22", "M4 18v-2", "M8 12v-2", "M8 18v-2",
      "M4 6h16a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2z"],
    net: ["m3 16 4 4 4-4", "M7 20V4", "m21 8-4-4-4 4", "M17 4v16"]
  };
  var READINGS = [
    { id: "cpu", name: T.cpu, color: "#5aa9ff", value: function () { return R.cpu; }, hist: function () { return R.cpuHist; } },
    { id: "gpu", name: T.gpu, color: "#41d27a", value: function () { return R.gpu; }, hist: function () { return R.gpuHist; } },
    { id: "mem", name: T.memory, color: "#ffad42", value: function () { return R.mem; }, hist: function () { return R.memHist; } }
  ];

  function icon(id, cls) {
    var s = svg("svg", { viewBox: "0 0 24 24", "class": "gw-icon " + (cls || id), "aria-hidden": "true" });
    ICONS[id].forEach(function (d) {
      s.appendChild(svg("path", { d: d, fill: "none", stroke: "currentColor", "stroke-width": 1.8, "stroke-linecap": "round", "stroke-linejoin": "round" }));
    });
    return s;
  }

  // A ring: the share of a whole as an arc round its icon.
  function ring(r) {
    var s = svg("svg", { viewBox: "0 0 40 40", "class": "gw-ring " + r.id, "aria-hidden": "true" });
    s.appendChild(svg("circle", { cx: 20, cy: 20, r: 16, "class": "track" }));
    var arc = svg("circle", { cx: 20, cy: 20, r: 16, "class": "arc", "stroke-dasharray": "0 101", transform: "rotate(-90 20 20)" });
    s.appendChild(arc);
    var g = svg("g", { transform: "translate(11 11) scale(0.75)" });
    ICONS[r.id].forEach(function (d) {
      g.appendChild(svg("path", { d: d, fill: "none", stroke: "currentColor", "stroke-width": 1.8, "stroke-linecap": "round", "stroke-linejoin": "round" }));
    });
    s.appendChild(g);
    return { node: s, update: function () { arc.setAttribute("stroke-dasharray", (r.value() * 1.005).toFixed(1) + " 101"); } };
  }

  // A figure with its icon: "45 %", or the network's rates.
  function figure(r, big) {
    var f = el("span", "gw-fig" + (big ? " big" : ""));
    f.appendChild(icon(r.id));
    var n = el("b");
    f.appendChild(n);
    f.appendChild(el("span", "u", "%"));
    return { node: f, update: function () { n.textContent = Math.round(r.value()); } };
  }
  function rates() {
    var f = el("span", "gw-fig net");
    f.appendChild(icon("net"));
    var n = el("span", "rate");
    f.appendChild(n);
    return { node: f, update: function () { n.textContent = "↓ " + demo.rate(R.down) + "  ↑ " + demo.rate(R.up); } };
  }

  // ---- the layouts, from the least room to the most ----
  function micro() {
    var box = el("div", "gw-micro");
    var parts = READINGS.map(ring);
    parts.forEach(function (p) { box.appendChild(p.node); });
    return { node: box, parts: parts };
  }
  function strip() {
    var box = el("div", "gw-strip");
    var parts = READINGS.map(function (r) { return figure(r); }).concat([rates()]);
    parts.forEach(function (p) { box.appendChild(p.node); });
    return { node: box, parts: parts };
  }
  function rail() {
    var box = el("div", "gw-rail");
    var c = chart(READINGS[0].color);
    box.appendChild(c.node);
    var parts = READINGS.map(function (r) { return figure(r); });
    parts.forEach(function (p) { box.appendChild(p.node); });
    parts.push({ update: function () { c.draw(R.cpuHist, 100); } });
    return { node: box, parts: parts };
  }
  function corner() {
    var box = el("div", "gw-corner");
    var plot = el("div", "plot");
    var cpu = chart(READINGS[0].color), gpu = chart(READINGS[1].color);
    gpu.node.classList.add("second");
    plot.appendChild(cpu.node);
    plot.appendChild(gpu.node);
    box.appendChild(plot);
    var row = el("div", "figs");
    var parts = READINGS.map(function (r) { return figure(r); }).concat([rates()]);
    parts.forEach(function (p) { row.appendChild(p.node); });
    box.appendChild(row);
    parts.push({ update: function () { cpu.draw(R.cpuHist, 100); gpu.draw(R.gpuHist, 100); } });
    return { node: box, parts: parts };
  }
  function tiles(columns) {
    var box = el("div", "gw-tiles");
    box.style.gridTemplateColumns = "repeat(" + columns + ", minmax(0, 1fr))";
    var parts = [];
    READINGS.forEach(function (r) {
      var t = el("div", "tile");
      var h = el("div", "th");
      h.appendChild(icon(r.id));
      h.appendChild(el("span", null, r.name));
      t.appendChild(h);
      var f = el("div", "tn");
      var n = el("b");
      f.appendChild(n);
      f.appendChild(el("span", "u", "%"));
      t.appendChild(f);
      var c = chart(r.color);
      t.appendChild(c.node);
      box.appendChild(t);
      parts.push({ update: function () { n.textContent = Math.round(r.value()); c.draw(r.hist(), 100); } });
    });
    var t = el("div", "tile");
    var h = el("div", "th");
    h.appendChild(icon("net"));
    h.appendChild(el("span", null, T.network));
    t.appendChild(h);
    var nd = el("div", "tr");
    t.appendChild(nd);
    var dc = chart("#c58bff");
    t.appendChild(dc.node);
    box.appendChild(t);
    parts.push({ update: function () {
      nd.textContent = "↓ " + demo.rate(R.down) + "  ↑ " + demo.rate(R.up);
      dc.draw(R.downHist, Math.max(4, Math.max.apply(null, R.downHist)));
    } });
    return { node: box, parts: parts };
  }
  function lanes(columns) {
    var box = el("div", "gw-lanes");
    box.style.gridTemplateColumns = "repeat(" + columns + ", minmax(0, 1fr))";
    var parts = [];
    READINGS.forEach(function (r) {
      var l = el("section", "lane");
      var h = el("div", "lh");
      h.appendChild(el("b", null, r.name));
      l.appendChild(h);
      var top = el("div", "top");
      var f = el("div", "big");
      var n = el("span", "n");
      f.appendChild(n);
      f.appendChild(el("span", "u", "%"));
      top.appendChild(f);
      var c = chart(r.color);
      top.appendChild(c.node);
      l.appendChild(top);
      box.appendChild(l);
      parts.push({ update: function () { n.textContent = Math.round(r.value()); c.draw(r.hist(), 100); } });
    });
    var l = el("section", "lane");
    var h = el("div", "lh");
    h.appendChild(el("b", null, T.network));
    l.appendChild(h);
    var top = el("div", "top");
    var rt = el("div", "tr");
    top.appendChild(rt);
    var dc = chart("#c58bff");
    top.appendChild(dc.node);
    l.appendChild(top);
    box.appendChild(l);
    parts.push({ update: function () {
      rt.textContent = "↓ " + demo.rate(R.down) + "  ↑ " + demo.rate(R.up);
      dc.draw(R.downHist, Math.max(4, Math.max.apply(null, R.downHist)));
    } });
    return { node: box, parts: parts };
  }

  // Which layout a room this large holds (px), as Glance's ladder chooses.
  function layoutFor(w, h) {
    if (w < 220) return h >= 240 ? "rail" : "micro";
    if (h < 120) return "strip";
    if (h < 300 || w < 340) return "corner";
    if (h < 460) return w >= 640 ? "tiles-4" : "tiles-2";
    return w >= 560 ? "lanes-2" : "lanes-1";
  }
  function make(form) {
    switch (form) {
      case "micro": return micro();
      case "strip": return strip();
      case "rail": return rail();
      case "corner": return corner();
      case "tiles-4": return tiles(4);
      case "tiles-2": return tiles(2);
      case "lanes-2": return lanes(2);
      default: return lanes(1);
    }
  }

  // ---- the widget, its corner to drag ----
  var box = el("div", "gw");
  var grip = el("div", "gw-grip");
  grip.setAttribute("aria-hidden", "true");
  host.appendChild(box);
  host.appendChild(grip);
  var size = { w: 340, h: 260 };
  var shown = null, form = null;

  function place() {
    var room = host.getBoundingClientRect();
    size.w = Math.max(110, Math.min(size.w, room.width - 32));
    size.h = Math.max(56, Math.min(size.h, room.height - 32));
    box.style.width = size.w + "px";
    box.style.height = size.h + "px";
    grip.style.left = (16 + size.w - 18) + "px";
    grip.style.top = (16 + size.h - 18) + "px";
    var next = layoutFor(size.w, size.h);
    if (next === form) return;
    form = next;
    // One layout turning into the next: the last fades as the new comes.
    var made = make(form);
    made.node.classList.add("gw-face");
    box.appendChild(made.node);
    if (shown) {
      var old = shown.node;
      old.classList.add("leaving");
      setTimeout(function () { if (old.parentNode) old.parentNode.removeChild(old); }, 300);
    }
    shown = made;
    update();
    requestAnimationFrame(function () { made.node.classList.add("in"); });
  }
  function update() { if (shown) shown.parts.forEach(function (p) { p.update(); }); }

  var from = null;
  grip.addEventListener("pointerdown", function (e) {
    from = { x: e.clientX, y: e.clientY, w: size.w, h: size.h };
    grip.setPointerCapture(e.pointerId);
    host.classList.add("sizing");
    e.preventDefault();
  });
  grip.addEventListener("pointermove", function (e) {
    if (!from) return;
    size.w = from.w + e.clientX - from.x;
    size.h = from.h + e.clientY - from.y;
    place();
  });
  function let_go() { from = null; host.classList.remove("sizing"); }
  grip.addEventListener("pointerup", let_go);
  grip.addEventListener("pointercancel", let_go);
  window.addEventListener("resize", place);

  demo.onTick(update);

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
    if (widget) place();
  }
  if (seg) {
    seg.addEventListener("click", function (e) {
      var b = e.target.closest("button[data-shape]");
      if (b) shape(b.getAttribute("data-shape"));
    });
    window.addEventListener("resize", function () { shape(host.hidden ? "panel" : "widget"); });
  }
  shape("panel");
  if (zh) host.setAttribute("aria-label", "可调整大小的小组件演示");
})();
