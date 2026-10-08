// The overlay drawn in the page, over a soft dusk in the colours of the
// desktop above it, running on made-up readings: a card or a strip on frosted glass, as Glance draws it.
(function () {
  "use strict";

  var stage = document.querySelector("[data-overlay-stage]");
  var seg = document.querySelector("[data-overlay-layout]");
  if (!stage || !seg) return;
  var zh = document.documentElement.lang.indexOf("zh") === 0;
  var SVG = "http://www.w3.org/2000/svg";
  var CHART = 60;

  var T = zh
    ? { fps: "帧率", cpu: "CPU", gpu: "GPU", mem: "内存", vram: "显存" }
    : { fps: "FPS", cpu: "CPU", gpu: "GPU", mem: "RAM", vram: "VRAM" };

  // ---- the readings: a game at a steady 144, now and then a stutter ----
  function walk(value, low, high, pull, jitter) {
    var mid = (low + high) / 2;
    return Math.min(high, Math.max(low, value + (mid - value) * pull + (Math.random() - 0.5) * jitter));
  }
  var R = { fps: 144, frames: [], cpu: 46, cpuTemp: 68, cpuPower: 96, gpu: 94, gpuTemp: 71, gpuPower: 214, mem: 18.4, vram: 9.1 };
  for (var i = 0; i < CHART; i++) R.frames.push(140 + Math.random() * 6);
  function step() {
    var stutter = Math.random() < 0.06;
    R.fps = Math.round(walk(R.fps, 138, 148, 0.4, 6) - (stutter ? 24 + Math.random() * 20 : 0));
    R.frames.push(R.fps);
    R.frames.shift();
    R.cpu = walk(R.cpu, 38, 56, 0.2, 8);
    R.cpuTemp = walk(R.cpuTemp, 64, 72, 0.2, 2);
    R.cpuPower = walk(R.cpuPower, 84, 110, 0.2, 8);
    R.gpu = walk(R.gpu, 90, 99, 0.3, 4);
    R.gpuTemp = walk(R.gpuTemp, 69, 74, 0.2, 1.5);
    R.gpuPower = walk(R.gpuPower, 200, 228, 0.2, 8);
    R.mem = walk(R.mem, 18, 19, 0.2, 0.2);
    R.vram = walk(R.vram, 8.9, 9.4, 0.2, 0.1);
  }
  function low() {
    var sorted = R.frames.slice().sort(function (a, b) { return a - b; });
    return Math.round(sorted[Math.floor(sorted.length * 0.01)]);
  }

  function el(tag, cls, parent, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    if (parent) parent.appendChild(e);
    return e;
  }
  function mark(kind, parent) { el("i", "ov-mark ov-" + kind, parent); }

  // A tile: a name, a big figure with its unit, and a line under it.
  function tile(kind, name, parent) {
    var t = el("div", "ov-tile", parent);
    var n = el("div", "ov-name", t);
    mark(kind, n);
    n.appendChild(document.createTextNode(name));
    var f = el("div", "ov-fig", t);
    var v = el("b", null, f);
    var u = el("span", "ov-unit", f);
    var rest = el("div", "ov-rest", t);
    return { v: v, u: u, rest: rest };
  }
  // A reading in the strip: a mark, a name, a figure, a unit and the rest.
  function item(kind, name, parent) {
    var t = el("div", "ov-item", parent);
    var n = el("span", "ov-name", t);
    mark(kind, n);
    n.appendChild(document.createTextNode(name));
    var v = el("b", null, t);
    var u = el("span", "ov-unit", t);
    var rest = el("span", "ov-rest", t);
    return { v: v, u: u, rest: rest };
  }

  // ---- the scene: a soft dusk in the colours of the desktop above it ----
  (function scene() {
    var W = 1200, H = 360;
    var svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("class", "art");
    svg.setAttribute("aria-hidden", "true");
    svg.setAttribute("viewBox", "0 0 " + W + " " + H);
    svg.setAttribute("preserveAspectRatio", "xMidYMax slice");
    stage.appendChild(svg);
    function node(tag, attrs, parent) {
      var e = document.createElementNS(SVG, tag);
      for (var k in attrs) e.setAttribute(k, attrs[k]);
      (parent || svg).appendChild(e);
      return e;
    }
    function gradient(id, stops) {
      var g = node("linearGradient", { id: id, x1: 0, y1: 0, x2: 0, y2: 1 }, defs);
      stops.forEach(function (s) { node("stop", { offset: s[0], "stop-color": s[1] }, g); });
    }
    var defs = node("defs", {});
    gradient("sc-sky", [["0", "#2a6ee0"], ["0.42", "#7f56d8"], ["0.7", "#c2559a"], ["0.9", "#ec8a4a"]]);
    gradient("sc-far", [["0", "#9a64d6"], ["1", "#6d48b8"]]);
    gradient("sc-mid", [["0", "#5a3aa6"], ["1", "#3f2a82"]]);
    gradient("sc-near", [["0", "#2f2468"], ["1", "#1d1745"]]);
    var sun = node("radialGradient", { id: "sc-sun" }, defs);
    [["0", "#fff0d2", "1"], ["0.18", "#ffd08a", "0.9"], ["0.5", "#ff9a52", "0.35"], ["1", "#ff9a52", "0"]].forEach(function (s) {
      node("stop", { offset: s[0], "stop-color": s[1], "stop-opacity": s[2] }, sun);
    });
    node("rect", { width: W, height: H, fill: "url(#sc-sky)" });
    // The sun on the horizon, half under the overlay's edge: frosted
    // there, clear past it.
    node("circle", { cx: 310, cy: 220, r: 210, fill: "url(#sc-sun)" });
    // Hills: smooth swells, far to near, each lower and darker.
    [["sc-far", 205, 26, 0.004, 0.0], ["sc-mid", 245, 30, 0.0055, 1.7], ["sc-near", 295, 24, 0.007, 3.1]].forEach(function (h) {
      var d = "M0 " + H;
      for (var x = 0; x <= W; x += 10) {
        var y = h[1] - h[2] * Math.sin(x * h[3] + h[4]) - h[2] * 0.5 * Math.sin(x * h[3] * 2.3 + h[4] * 1.9);
        d += "L" + x + " " + y.toFixed(1);
      }
      node("path", { d: d + "L" + W + " " + H + "Z", fill: "url(#" + h[0] + ")" });
    });
  })();

  var glass = el("div", "ov-glass", stage);
  var parts = {};

  function build(layout) {
    glass.replaceChildren();
    glass.className = "ov-glass ov-" + layout;
    parts = {};
    if (layout === "card") {
      var hero = el("div", "ov-hero", glass);
      var left = el("div", null, hero);
      var n = el("div", "ov-name", left);
      mark("fps", n);
      n.appendChild(document.createTextNode(T.fps));
      parts.fps = el("div", "ov-big", left);
      parts.fpsRest = el("div", "ov-rest", left);
      var svg = document.createElementNS(SVG, "svg");
      svg.setAttribute("viewBox", "0 0 120 48");
      svg.setAttribute("preserveAspectRatio", "none");
      svg.setAttribute("aria-hidden", "true");
      // The area under the line fades out downward, as Glance draws it.
      var defs = document.createElementNS(SVG, "defs");
      var fade = document.createElementNS(SVG, "linearGradient");
      fade.setAttribute("id", "ov-fade");
      fade.setAttribute("x1", "0"); fade.setAttribute("y1", "0"); fade.setAttribute("x2", "0"); fade.setAttribute("y2", "1");
      [["0", "0.3"], ["1", "0"]].forEach(function (s) {
        var stop = document.createElementNS(SVG, "stop");
        stop.setAttribute("offset", s[0]);
        stop.setAttribute("stop-color", "#8be08f");
        stop.setAttribute("stop-opacity", s[1]);
        fade.appendChild(stop);
      });
      defs.appendChild(fade);
      svg.appendChild(defs);
      parts.area = document.createElementNS(SVG, "path");
      parts.area.setAttribute("class", "ov-area");
      parts.line = document.createElementNS(SVG, "path");
      parts.line.setAttribute("class", "ov-line");
      svg.appendChild(parts.area);
      svg.appendChild(parts.line);
      hero.appendChild(svg);
      var tiles = el("div", "ov-tiles", glass);
      parts.cpu = tile("cpu", T.cpu, tiles);
      parts.gpu = tile("gpu", T.gpu, tiles);
      parts.mem = tile("mem", T.mem, tiles);
      parts.vram = tile("mem", T.vram, tiles);
    } else {
      parts.fpsItem = item("fps", T.fps, glass);
      el("i", "ov-rule", glass);
      parts.cpu = item("cpu", T.cpu, glass);
      el("i", "ov-rule", glass);
      parts.gpu = item("gpu", T.gpu, glass);
      el("i", "ov-rule", glass);
      parts.mem = item("mem", T.mem, glass);
    }
    paint();
  }

  function chart() {
    var most = Math.max.apply(null, R.frames), least = Math.min.apply(null, R.frames);
    var span = Math.max(most - least, most * 0.1), bottom = most - span * 1.1;
    var d = R.frames.map(function (f, i) {
      var x = (i / (CHART - 1)) * 120, y = 2 + (1 - (f - bottom) / (most - bottom)) * 44;
      return (i ? "L" : "M") + x.toFixed(1) + " " + y.toFixed(1);
    }).join("");
    parts.line.setAttribute("d", d);
    parts.area.setAttribute("d", d + "L120 48L0 48Z");
  }

  function set(p, v, u, rest) {
    p.v.textContent = v;
    p.u.textContent = u;
    p.rest.textContent = rest;
  }

  function paint() {
    var c = Math.round(R.cpu), g = Math.round(R.gpu);
    if (parts.fps) {
      parts.fps.textContent = R.fps;
      parts.fpsRest.textContent = "1% " + low() + " · " + (1000 / R.fps).toFixed(1) + " ms";
      chart();
      set(parts.cpu, c, "%", Math.round(R.cpuTemp) + " °C · " + Math.round(R.cpuPower) + " W");
      set(parts.gpu, g, "%", Math.round(R.gpuTemp) + " °C · " + Math.round(R.gpuPower) + " W");
      set(parts.mem, R.mem.toFixed(1), "GB", zh ? "共 32 GB" : "of 32 GB");
      set(parts.vram, R.vram.toFixed(1), "GB", zh ? "共 16 GB" : "of 16 GB");
    } else {
      set(parts.fpsItem, R.fps, "", "1% " + low());
      set(parts.cpu, c, "%", Math.round(R.cpuTemp) + " °C");
      set(parts.gpu, g, "%", Math.round(R.gpuTemp) + " °C");
      set(parts.mem, R.mem.toFixed(1), "GB", "");
    }
  }

  // ---- the layout switch ----
  var buttons = seg.querySelectorAll("button");
  function choose(layout) {
    buttons.forEach(function (b) {
      var on = b.getAttribute("data-layout") === layout;
      b.setAttribute("aria-pressed", on ? "true" : "false");
      if (on) {
        seg.style.setProperty("--x", b.offsetLeft + "px");
        seg.style.setProperty("--w", b.offsetWidth + "px");
      }
    });
    build(layout);
  }
  buttons.forEach(function (b) {
    b.addEventListener("click", function () { choose(b.getAttribute("data-layout")); });
  });
  window.addEventListener("resize", function () {
    var on = seg.querySelector('[aria-pressed="true"]');
    if (on) {
      seg.style.setProperty("--x", on.offsetLeft + "px");
      seg.style.setProperty("--w", on.offsetWidth + "px");
    }
  });
  choose("card");

  var still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (!still) setInterval(function () { step(); paint(); }, 1000);
})();
