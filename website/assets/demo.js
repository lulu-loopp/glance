// A Glance panel drawn in the page, running on made-up readings: it slides
// out when the pointer is pushed against the window's right edge, as Glance
// does at the screen's, and shows each of its looks in place.
(function () {
  "use strict";

  var root = document.documentElement;
  var zh = root.lang.indexOf("zh") === 0;
  var SVG = "http://www.w3.org/2000/svg";
  var HISTORY = 60;
  var THREADS = 32;

  var T = zh ? {
    cpu: "CPU", gpu: "GPU", memory: "内存", network: "网络", processes: "进程",
    clock: "频率", power: "功耗", ccd: "CCD", vram: "显存", fan: "风扇",
    down: "下载", up: "上传", cols: ["CPU", "内存", "GPU"],
    uptime: function (m) { return "已开机 " + Math.floor(m / 60) + " 小时 " + (m % 60) + " 分"; }
  } : {
    cpu: "CPU", gpu: "GPU", memory: "Memory", network: "Network", processes: "Processes",
    clock: "Clock", power: "Power", ccd: "CCD", vram: "VRAM", fan: "Fan",
    down: "Down", up: "Up", cols: ["CPU", "Mem", "GPU"],
    uptime: function (m) { return "Up " + Math.floor(m / 60) + " h " + (m % 60) + " min"; }
  };

  // ---- the readings: slow walks, as a machine at moderate work ----
  function walk(value, low, high, pull, jitter) {
    var mid = (low + high) / 2;
    var next = value + (mid - value) * pull + (Math.random() - 0.5) * jitter;
    return Math.min(high, Math.max(low, next));
  }
  function filled(n, v) { var a = []; for (var i = 0; i < n; i++) a.push(v); return a; }

  var R = {
    cpu: 42, cpuHist: filled(HISTORY, 42), clock: 4.9, power: 108, ccd1: 61, ccd2: 45, cpuTemp: 59,
    threads: filled(THREADS, 0.4),
    gpu: 56, gpuHist: filled(HISTORY, 56), gpuClock: 2572, gpuPower: 162, fan: 1385, gpuTemp: 55, vram: 6.6,
    mem: 37, memHist: filled(HISTORY, 37),
    down: 3.0, up: 0.02, downHist: filled(HISTORY, 0.5), upHist: filled(HISTORY, 0.05),
    procs: [
      { name: "blender", cpu: 30, mem: 2.5, gpu: 33 },
      { name: "chrome", cpu: 17, mem: 1.7, gpu: 0.8 },
      { name: "code", cpu: 12, mem: 1.4, gpu: 0.6 },
      { name: "obs64", cpu: 12, mem: 1.2, gpu: 0.4 },
      { name: "glance", cpu: 0.9, mem: 0.1, gpu: 0.2 }
    ],
    uptime: 206
  };

  function push(list, v) { list.push(v); list.shift(); }

  function step() {
    R.cpu = walk(R.cpu, 18, 72, 0.08, 14);
    push(R.cpuHist, R.cpu);
    R.clock = walk(R.clock, 4.6, 5.6, 0.2, 0.25);
    R.power = 40 + R.cpu * 1.6 + (Math.random() - 0.5) * 6;
    R.ccd1 = walk(R.ccd1, 48, 74, 0.1, 3);
    R.ccd2 = walk(R.ccd2, 40, 62, 0.1, 2.4);
    R.cpuTemp = Math.max(R.ccd1, R.ccd2) - 2;
    var base = R.cpu / 100;
    R.threads = R.threads.map(function (t, i) {
      var bias = i % 2 ? 0.75 : 1.15;
      return Math.min(1, Math.max(0.03, t + (base * bias - t) * 0.5 + (Math.random() - 0.5) * 0.35));
    });
    R.gpu = walk(R.gpu, 30, 92, 0.1, 16);
    push(R.gpuHist, R.gpu);
    R.gpuClock = 2200 + R.gpu * 5 + (Math.random() - 0.5) * 30;
    R.gpuPower = 30 + R.gpu * 2.4 + (Math.random() - 0.5) * 8;
    R.gpuTemp = walk(R.gpuTemp, 46, 66, 0.08, 1.6);
    R.fan = 900 + R.gpuTemp * 9;
    R.vram = walk(R.vram, 5.8, 7.4, 0.1, 0.2);
    R.mem = walk(R.mem, 35, 40, 0.15, 0.8);
    push(R.memHist, R.mem);
    // Bursts on the network, as downloads come and go.
    R.down = Math.random() < 0.2 ? 1 + Math.random() * 9 : Math.max(0.05, R.down * (0.45 + Math.random() * 0.4));
    R.up = Math.random() < 0.15 ? 0.2 + Math.random() * 1.2 : Math.max(0.01, R.up * 0.5);
    push(R.downHist, R.down);
    push(R.upHist, R.up);
    R.procs.forEach(function (p, i) {
      if (p.name === "glance") return;
      p.cpu = walk(p.cpu, [18, 8, 4, 6][i], [42, 26, 20, 16][i], 0.2, 6);
      p.gpu = p.name === "blender" ? walk(p.gpu, 20, 60, 0.2, 10) : walk(p.gpu, 0.2, 2, 0.3, 0.4);
    });
    R.procs.sort(function (a, b) { return b.cpu - a.cpu; });
  }

  // ---- drawing ----
  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  function svg(tag, attrs) {
    var e = document.createElementNS(SVG, tag);
    for (var k in attrs) e.setAttribute(k, attrs[k]);
    return e;
  }

  var uid = 0;

  // A chart: the line over a fill fading down, on a faint grid.
  function chart(color) {
    var id = "g" + (++uid);
    var s = svg("svg", { viewBox: "0 0 200 44", preserveAspectRatio: "none", "class": "chart", "aria-hidden": "true" });
    var grad = svg("linearGradient", { id: id, x1: "0", y1: "0", x2: "0", y2: "1" });
    grad.appendChild(svg("stop", { offset: "0", "stop-color": color, "stop-opacity": "0.35" }));
    grad.appendChild(svg("stop", { offset: "1", "stop-color": color, "stop-opacity": "0" }));
    var defs = svg("defs", {});
    defs.appendChild(grad);
    s.appendChild(defs);
    for (var x = 40; x < 200; x += 40) s.appendChild(svg("line", { x1: x, y1: 0, x2: x, y2: 44, "class": "grid" }));
    var area = svg("path", { fill: "url(#" + id + ")" });
    var line = svg("polyline", { fill: "none", stroke: color, "stroke-width": "1.6", "vector-effect": "non-scaling-stroke", "stroke-linejoin": "round" });
    var dot = svg("circle", { r: "2.2", fill: color, "class": "dot" });
    s.appendChild(area);
    s.appendChild(line);
    s.appendChild(dot);
    return {
      node: s,
      draw: function (values, max) {
        var pts = values.map(function (v, i) {
          return (i * 200 / (values.length - 1)).toFixed(1) + "," + (42 - Math.min(1, v / max) * 38).toFixed(1);
        });
        line.setAttribute("points", pts.join(" "));
        area.setAttribute("d", "M0,44 L" + pts.join(" L") + " L200,44 Z");
        var last = pts[pts.length - 1].split(",");
        dot.setAttribute("cx", last[0]);
        dot.setAttribute("cy", last[1]);
      }
    };
  }

  // A lane's head: its name, the part's model in grey, and a figure at the right.
  function head(name, model) {
    var h = el("div", "lh");
    h.appendChild(el("b", null, name));
    h.appendChild(el("span", "model", model));
    var right = el("span", "hr");
    h.appendChild(right);
    return { node: h, right: right };
  }

  function big() {
    var b = el("div", "big");
    var n = el("span", "n");
    b.appendChild(n);
    b.appendChild(el("span", "u", "%"));
    return { node: b, n: n };
  }

  function rows(names) {
    var box = el("dl", "rows");
    var cells = names.map(function (name) {
      box.appendChild(el("dt", null, name));
      var dd = el("dd");
      box.appendChild(dd);
      return dd;
    });
    return { node: box, cells: cells };
  }

  function bar(color) {
    var s = svg("svg", { viewBox: "0 0 100 4", preserveAspectRatio: "none", "class": "bar", "aria-hidden": "true" });
    s.appendChild(svg("rect", { x: 0, y: 0, width: 100, height: 4, rx: 2, "class": "track" }));
    var fill = svg("rect", { x: 0, y: 0, width: 0, height: 4, rx: 2, fill: color });
    s.appendChild(fill);
    return { node: s, set: function (f) { fill.setAttribute("width", (f * 100).toFixed(1)); } };
  }

  function threadBars() {
    var s = svg("svg", { viewBox: "0 0 " + (THREADS * 6) + " 12", preserveAspectRatio: "none", "class": "threads", "aria-hidden": "true" });
    var bars = [];
    for (var i = 0; i < THREADS; i++) {
      s.appendChild(svg("rect", { x: i * 6, y: 0, width: 4.6, height: 12, rx: 1, "class": "track" }));
      var b = svg("rect", { x: i * 6, y: 12, width: 4.6, height: 0, rx: 1, "class": "on" });
      s.appendChild(b);
      bars.push(b);
    }
    return {
      node: s,
      set: function (values) {
        values.forEach(function (v, i) {
          var h = Math.max(1.5, v * 12);
          bars[i].setAttribute("y", (12 - h).toFixed(2));
          bars[i].setAttribute("height", h.toFixed(2));
        });
      }
    };
  }

  function rate(mb) {
    return mb >= 1 ? mb.toFixed(1) + " MB/s" : Math.round(mb * 1024) + " KB/s";
  }

  // One panel: its lanes in two columns, as Glance lays them out when one
  // column would not fit, and a function that brings them up to the readings.
  function panel() {
    var p = el("div", "gp");
    var cols = [el("div", "col"), el("div", "col")];
    cols.forEach(function (c) { p.appendChild(c); });
    var updates = [];
    function lane(cls, col) { var l = el("section", "lane " + cls); cols[col].appendChild(l); return l; }

    // CPU
    var cpu = lane("cpu", 0);
    var ch = head(T.cpu, "AMD Ryzen 9 9950X");
    var cb = big();
    var cc = chart("#5aa9ff");
    var top = el("div", "top");
    top.appendChild(cb.node);
    top.appendChild(cc.node);
    var cr = rows([T.clock, T.power, T.ccd + " 1", T.ccd + " 2"]);
    var tb = threadBars();
    [ch.node, top, cr.node, tb.node].forEach(function (n) { cpu.appendChild(n); });
    updates.push(function () {
      ch.right.textContent = Math.round(R.cpuTemp) + " °C";
      cb.n.textContent = Math.round(R.cpu);
      cc.draw(R.cpuHist, 100);
      cr.cells[0].textContent = R.clock.toFixed(2) + " GHz";
      cr.cells[1].textContent = R.power.toFixed(1) + " W";
      cr.cells[2].textContent = Math.round(R.ccd1) + " °C";
      cr.cells[3].textContent = Math.round(R.ccd2) + " °C";
      tb.set(R.threads);
    });

    // GPU
    var gpu = lane("gpu", 0);
    var gh = head(T.gpu, "NVIDIA GeForce RTX 5070 Ti");
    var gb = big();
    var gc = chart("#41d27a");
    var gtop = el("div", "top");
    gtop.appendChild(gb.node);
    gtop.appendChild(gc.node);
    var vline = el("div", "vram");
    vline.appendChild(el("span", "k", T.vram));
    var vb = bar("#41d27a");
    vline.appendChild(vb.node);
    var vt = el("span", "v");
    vline.appendChild(vt);
    var gr = rows([T.clock, T.power, T.fan]);
    [gh.node, gtop, vline, gr.node].forEach(function (n) { gpu.appendChild(n); });
    updates.push(function () {
      gh.right.textContent = Math.round(R.gpuTemp) + " °C";
      gb.n.textContent = Math.round(R.gpu);
      gc.draw(R.gpuHist, 100);
      vb.set(R.vram / 16);
      vt.textContent = R.vram.toFixed(1) + " / 16 GB";
      gr.cells[0].textContent = Math.round(R.gpuClock) + " MHz";
      gr.cells[1].textContent = R.gpuPower.toFixed(1) + " W";
      gr.cells[2].textContent = Math.round(R.fan) + " RPM";
    });

    // Memory
    var mem = lane("mem", 1);
    var mh = head(T.memory, "2 × 32 GB DDR5-6000");
    var mb = big();
    var mc = chart("#ffad42");
    var mtop = el("div", "top");
    mtop.appendChild(mb.node);
    mtop.appendChild(mc.node);
    [mh.node, mtop].forEach(function (n) { mem.appendChild(n); });
    updates.push(function () {
      mh.right.textContent = (R.mem / 100 * 61.6).toFixed(1) + " / 61.6 GB";
      mb.n.textContent = Math.round(R.mem);
      mc.draw(R.memHist, 100);
    });

    // Network
    var net = lane("net", 1);
    var nh = head(T.network, "Wi-Fi 6E");
    var nrow = el("div", "top");
    var legend = el("dl", "rates");
    legend.appendChild(el("dt", "down", T.down));
    var dv = el("dd");
    legend.appendChild(dv);
    legend.appendChild(el("dt", "up", T.up));
    var uv = el("dd");
    legend.appendChild(uv);
    var nwrap = el("div", "pair");
    var dc = chart("#c58bff");
    var uc = chart("#5fd6e6");
    nwrap.appendChild(dc.node);
    nwrap.appendChild(uc.node);
    nrow.appendChild(legend);
    nrow.appendChild(nwrap);
    [nh.node, nrow].forEach(function (n) { net.appendChild(n); });
    updates.push(function () {
      nh.right.textContent = "";
      dv.textContent = rate(R.down);
      uv.textContent = rate(R.up);
      var max = Math.max(4, Math.max.apply(null, R.downHist));
      dc.draw(R.downHist, max);
      uc.draw(R.upHist, max);
    });

    // Processes
    var pr = lane("procs", 1);
    var table = el("table");
    var thead = el("tr");
    thead.appendChild(el("th", null, T.processes));
    T.cols.forEach(function (c) { thead.appendChild(el("th", null, c)); });
    table.appendChild(thead);
    var prow = [];
    for (var i = 0; i < 5; i++) {
      var tr = el("tr");
      var cells = [el("td"), el("td"), el("td"), el("td")];
      cells.forEach(function (c) { tr.appendChild(c); });
      table.appendChild(tr);
      prow.push(cells);
    }
    pr.appendChild(table);
    updates.push(function () {
      R.procs.forEach(function (p, i) {
        prow[i][0].textContent = p.name;
        prow[i][1].textContent = (p.cpu < 10 ? p.cpu.toFixed(1) : Math.round(p.cpu)) + "%";
        prow[i][2].textContent = p.mem.toFixed(1) + " GB";
        prow[i][3].textContent = (p.gpu < 10 ? p.gpu.toFixed(1) : Math.round(p.gpu)) + "%";
      });
    });

    // The foot: how long the machine has been up, and the settings' gear.
    var foot = el("div", "lane foot");
    var up = el("span");
    foot.appendChild(up);
    var gear = svg("svg", { viewBox: "0 0 24 24", "class": "gear", "aria-hidden": "true" });
    gear.appendChild(svg("circle", { cx: 12, cy: 12, r: 3, fill: "none", stroke: "currentColor", "stroke-width": 1.6 }));
    gear.appendChild(svg("path", { d: "M12 2.5v3M12 18.5v3M2.5 12h3M18.5 12h3M5.3 5.3l2.1 2.1M16.6 16.6l2.1 2.1M5.3 18.7l2.1-2.1M16.6 7.4l2.1-2.1", fill: "none", stroke: "currentColor", "stroke-width": 1.6, "stroke-linecap": "round" }));
    foot.appendChild(gear);
    p.appendChild(foot);
    updates.push(function () { up.textContent = T.uptime(R.uptime); });

    return { node: p, update: function () { updates.forEach(function (u) { u(); }); } };
  }

  // ---- the panels on the page ----
  var panels = [];
  Array.prototype.forEach.call(document.querySelectorAll("[data-demo]"), function (host) {
    var p = panel();
    host.appendChild(p.node);
    panels.push(p);
  });
  if (!panels.length) return;

  function refresh() { panels.forEach(function (p) { p.update(); }); }
  // A minute of readings already behind the charts, as in a panel just opened.
  for (var i = 0; i < HISTORY; i++) step();
  refresh();
  var ticks = 0;
  setInterval(function () {
    // Off screen or in a hidden tab, nothing to show.
    if (document.hidden) return;
    step();
    if (++ticks % 60 === 0) R.uptime++;
    refresh();
  }, 1000);

  // ---- the looks: one choice for every panel on the page ----
  var seg = document.querySelector("[data-looks]");
  function look(name) {
    root.setAttribute("data-look", name);
    if (seg) Array.prototype.forEach.call(seg.querySelectorAll("button"), function (b) {
      var on = b.getAttribute("data-look") === name;
      b.setAttribute("aria-pressed", on ? "true" : "false");
      if (on) {
        seg.style.setProperty("--x", b.offsetLeft + "px");
        seg.style.setProperty("--w", b.offsetWidth + "px");
      }
    });
  }
  if (seg) {
    seg.addEventListener("click", function (e) {
      var b = e.target.closest("button[data-look]");
      if (b) look(b.getAttribute("data-look"));
    });
    window.addEventListener("resize", function () { look(root.getAttribute("data-look")); });
  }
  look("glass");

  // ---- the edge: push the pointer against the window's right edge ----
  var drawer = document.querySelector("[data-drawer]");
  if (!drawer) return;
  var hint = document.querySelector("[data-edge]");
  var peek = document.querySelectorAll("[data-peek]");
  var open = false, closing = null;

  function setOpen(on) {
    if (closing) { clearTimeout(closing); closing = null; }
    if (on === open) return;
    open = on;
    drawer.classList.toggle("open", on);
    drawer.setAttribute("aria-hidden", on ? "false" : "true");
    if (hint) hint.classList.toggle("gone", on);
  }

  // At the edge, the pointer can go no further: as in Glance, that is the push.
  document.addEventListener("pointermove", function (e) {
    if (e.pointerType !== "mouse") return;
    var edge = document.documentElement.clientWidth;
    if (e.clientX >= edge - 3) { setOpen(true); return; }
    if (!open) return;
    var r = drawer.getBoundingClientRect();
    var near = e.clientX >= r.left - 32 && e.clientY >= r.top - 32 && e.clientY <= r.bottom + 32;
    if (near) { if (closing) { clearTimeout(closing); closing = null; } }
    else if (!closing) closing = setTimeout(function () { setOpen(false); }, 220);
  }, { passive: true });
  document.documentElement.addEventListener("mouseleave", function (e) {
    // Leaving through the right edge is the same push, when the browser fills the screen.
    if (e.clientX >= document.documentElement.clientWidth - 3) setOpen(true);
  });
  document.addEventListener("keydown", function (e) { if (e.key === "Escape") setOpen(false); });
  Array.prototype.forEach.call(peek, function (b) {
    b.addEventListener("click", function (e) { e.stopPropagation(); setOpen(!open); });
  });
  document.addEventListener("click", function (e) {
    if (open && !drawer.contains(e.target)) setOpen(false);
  });
})();
