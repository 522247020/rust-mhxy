/*
 * swarm-intro.js — 静态页开场「多光标重建」动画
 *
 * 页面加载后，先把当前视口清空，再让一群带名字的多人光标把页面重新
 * 画出来：环绕图标集合、拉框、打字、画分隔线、揭开图片。完成后淡回
 * 真实页面。全程无依赖，适配站点的主题色。
 *
 * 灵感来自 Swarm（Joel Califa）与 swarm-cursors（Dhvanil Patel），
 * 但这里是独立实现，没有复用它们的代码。
 *
 * 用法：
 *   <script src="swarm-intro.js" defer></script>
 *
 * 行为：
 *   - 每个浏览器会话只自动播放一次（sessionStorage）
 *   - 窄屏 / prefers-reduced-motion / iframe 中不自动播放
 *   - <html data-no-swarm> 或 window.SWARM_INTRO.auto = false 可关闭自动播放
 *   - ?swarm=1 或地址栏 #swarm 可强制播放
 *   - 控制台执行 SwarmIntro.play() 可重播
 *   - Esc 或底部「跳过」立即结束
 *
 * 可选配置（在脚本之前）：
 *   window.SWARM_INTRO = { maxCursors: 16, maxActive: 8, speed: 1, oncePerSession: true, auto: true };
 */
(function () {
  "use strict";

  if (window.__swarmIntroLoaded) return;
  window.__swarmIntroLoaded = true;

  const DEFAULTS = {
    minWidth: 720,
    maxCursors: 16,
    maxActive: 8,
    speed: 1,
    oncePerSession: true,
    sessionKey: "swarm-intro-played",
  };
  const cfg = Object.assign({}, DEFAULTS, window.SWARM_INTRO || {});

  const NAMES = [
    "阿绿", "小金", "Ada", "小满", "Grace", "阿蓝", "Ken", "Nova",
    "林", "Mika", "阿橙", "Linus", "桃子", "Yuki", "Vint", "阿紫",
  ];
  const COLORS = [
    "#63dab7", "#efc271", "#8ab4ff", "#f48fb1", "#b39dff", "#6ee7b7",
    "#ffd166", "#7dd3fc", "#fca5a5", "#c4b5fd", "#5eead4", "#fdba74",
    "#a5b4fc", "#f9a8d4", "#86efac", "#fcd34d",
  ];

  const REDUCED = !!(window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  const FORCED = /(?:^|[?&])swarm=1(?:&|$)/.test(location.search) || location.hash === "#swarm";
  const IN_FRAME = (() => {
    try { return window.top !== window.self; } catch (_) { return true; }
  })();
  let SEEN = false;
  try { SEEN = sessionStorage.getItem(cfg.sessionKey) === "1"; } catch (_) {}

  const AUTO_OK =
    !IN_FRAME &&
    !document.documentElement.hasAttribute("data-no-swarm") &&
    (FORCED || (!REDUCED && innerWidth >= cfg.minWidth && !(cfg.oncePerSession && SEEN)));

  // ------------------------------------------------------------------ helpers

  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const lerp = (a, b, t) => a + (b - a) * t;
  const rand = (a, b) => a + Math.random() * (b - a);
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const nextFrame = () => new Promise((resolve) => requestAnimationFrame(resolve));
  const easeOutCubic = (t) => 1 - Math.pow(1 - t, 3);
  const easeInOutCubic = (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);

  function parseColor(input) {
    if (!input) return { r: 0, g: 0, b: 0, a: 0 };
    const s = String(input).trim();
    if (s === "transparent") return { r: 0, g: 0, b: 0, a: 0 };
    if (s[0] === "#") {
      let h = s.slice(1);
      if (h.length === 3 || h.length === 4) h = h.split("").map((c) => c + c).join("");
      if (h.length !== 6 && h.length !== 8) return { r: 0, g: 0, b: 0, a: 0 };
      const n = parseInt(h.slice(0, 6), 16);
      return {
        r: (n >> 16) & 255,
        g: (n >> 8) & 255,
        b: n & 255,
        a: h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1,
      };
    }
    const m = s.match(/rgba?\(([^)]+)\)/i);
    if (!m) return { r: 0, g: 0, b: 0, a: 0 };
    const p = m[1].split(/[,\s/]+/).filter(Boolean).map(Number);
    return {
      r: p[0] || 0,
      g: p[1] || 0,
      b: p[2] || 0,
      a: p.length > 3 ? (p[3] <= 1 ? p[3] : p[3] / 255) : 1,
    };
  }

  const rgba = (c, a) => `rgba(${Math.round(c.r)}, ${Math.round(c.g)}, ${Math.round(c.b)}, ${a})`;

  function withAlpha(color, alpha) {
    const c = parseColor(color);
    if (!c.r && !c.g && !c.b && !c.a) return color;
    return rgba(c, (c.a || 1) * alpha);
  }

  function cssVar(name, fallback) {
    const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return value || fallback;
  }

  function fontOf(cs) {
    const style = cs.fontStyle && cs.fontStyle !== "normal" ? cs.fontStyle + " " : "";
    return `${style}${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
  }

  function radiiOf(cs) {
    const read = (v) => parseFloat(v) || 0;
    return [
      read(cs.borderTopLeftRadius),
      read(cs.borderTopRightRadius),
      read(cs.borderBottomRightRadius),
      read(cs.borderBottomLeftRadius),
    ];
  }

  function hasDirectText(el) {
    for (let n = el.firstChild; n; n = n.nextSibling) {
      if (n.nodeType === 3 && n.nodeValue && n.nodeValue.trim()) return true;
    }
    return false;
  }

  function textColorFor(bg) {
    const c = parseColor(bg);
    return 0.299 * c.r + 0.587 * c.g + 0.114 * c.b > 145 ? "#0b1218" : "#ffffff";
  }

  function roundRect(ctx, x, y, w, h, radii) {
    const [a, b, c, d] = radii;
    const tl = Math.min(a, w / 2, h / 2);
    const tr = Math.min(b, w / 2, h / 2);
    const br = Math.min(c, w / 2, h / 2);
    const bl = Math.min(d, w / 2, h / 2);
    ctx.beginPath();
    ctx.moveTo(x + tl, y);
    ctx.lineTo(x + w - tr, y);
    ctx.arcTo(x + w, y, x + w, y + tr, tr);
    ctx.lineTo(x + w, y + h - br);
    ctx.arcTo(x + w, y + h, x + w - br, y + h, br);
    ctx.lineTo(x + bl, y + h);
    ctx.arcTo(x, y + h, x, y + h - bl, bl);
    ctx.lineTo(x, y + tl);
    ctx.arcTo(x, y, x + tl, y, tl);
    ctx.closePath();
  }

  function paintBox(ctx, item, alpha) {
    const r = item.rect;
    const a = alpha == null ? 1 : alpha;
    if (a <= 0.001) return;
    const prevAlpha = ctx.globalAlpha;
    if (item.fill && item.fill.a > 0.002) {
      ctx.globalAlpha = prevAlpha * item.fill.a * a;
      ctx.fillStyle = rgba(item.fill, 1);
      roundRect(ctx, r.x, r.y, r.w, r.h, item.radius);
      ctx.fill();
    }
    if (item.border && item.border.w > 0) {
      const inset = item.border.w / 2;
      ctx.globalAlpha = prevAlpha * a * (item.border.color.a || 1);
      ctx.strokeStyle = rgba(item.border.color, 1);
      ctx.lineWidth = item.border.w;
      roundRect(
        ctx,
        r.x + inset,
        r.y + inset,
        Math.max(1, r.w - inset * 2),
        Math.max(1, r.h - inset * 2),
        item.radius.map((v) => Math.max(0, v - inset)),
      );
      ctx.stroke();
    }
    ctx.globalAlpha = prevAlpha;
  }

  function paintRule(ctx, item, from, to) {
    ctx.strokeStyle = rgba(item.color, item.color.a || 1);
    ctx.lineWidth = item.width;
    ctx.beginPath();
    ctx.moveTo(from, item.y);
    ctx.lineTo(to, item.y);
    ctx.stroke();
  }

  // -------------------------------------------------------------------- host

  const CSS = `
    :host { all: initial; }
    .stage {
      position: absolute;
      inset: 0;
      overflow: hidden;
      opacity: 0;
      background: #0f141c;
      transition: opacity .42s ease;
    }
    .stage.in { opacity: 1; }
    .stage.out { opacity: 0; transition: opacity .55s ease; }
    canvas {
      position: absolute;
      inset: 0;
      width: 100%;
      height: 100%;
      display: block;
    }
    .cursors { position: absolute; inset: 0; z-index: 1; overflow: hidden; pointer-events: none; }
    .cursor {
      position: absolute;
      left: 0;
      top: 0;
      opacity: 0;
      will-change: transform;
      transition: opacity .3s ease;
      pointer-events: none;
    }
    .cursor svg {
      display: block;
      width: 19px;
      height: 19px;
      filter: drop-shadow(0 2px 3px rgba(0, 0, 0, .35));
    }
    .tag {
      position: absolute;
      left: 10px;
      top: 14px;
      padding: 2px 6px;
      border-radius: 4px;
      font: 600 11px/1.35 "Microsoft YaHei UI", "PingFang SC", system-ui, sans-serif;
      letter-spacing: .01em;
      white-space: nowrap;
      box-shadow: 0 2px 8px rgba(0, 0, 0, .22);
      transition: opacity .25s ease;
    }
    .hud {
      position: absolute;
      z-index: 2;
      left: 50%;
      bottom: 24px;
      display: flex;
      align-items: center;
      gap: 10px;
      max-width: calc(100% - 32px);
      padding: 8px 9px 8px 14px;
      border: 1px solid rgba(255, 255, 255, .08);
      border-radius: 999px;
      background: rgba(12, 17, 24, .84);
      color: #e5ecf3;
      font: 500 12px/1.2 "Microsoft YaHei UI", "PingFang SC", system-ui, sans-serif;
      white-space: nowrap;
      box-shadow: 0 10px 34px rgba(0, 0, 0, .38);
      backdrop-filter: blur(9px);
      -webkit-backdrop-filter: blur(9px);
      opacity: 0;
      transform: translate(-50%, 10px);
      transition: opacity .35s ease, transform .35s ease;
      pointer-events: none;
    }
    .hud.on { opacity: 1; transform: translate(-50%, 0); pointer-events: auto; }
    .hud .live {
      width: 7px;
      height: 7px;
      flex: none;
      border-radius: 50%;
      background: #63dab7;
      box-shadow: 0 0 0 3px rgba(99, 218, 183, .14);
    }
    .hud .txt { overflow: hidden; text-overflow: ellipsis; }
    .hud .bar {
      width: 108px;
      height: 3px;
      flex: none;
      overflow: hidden;
      border-radius: 99px;
      background: rgba(255, 255, 255, .12);
    }
    .hud .bar i {
      display: block;
      width: 0;
      height: 100%;
      border-radius: inherit;
      background: #63dab7;
      transition: width .25s ease;
    }
    .hud .skip {
      flex: none;
      padding: 4px 9px;
      border: 0;
      border-radius: 99px;
      background: rgba(255, 255, 255, .08);
      color: #b9c6d4;
      font: inherit;
      cursor: pointer;
    }
    .hud .skip:hover { background: rgba(255, 255, 255, .15); color: #fff; }
    .hud .skip kbd {
      margin-left: 5px;
      padding: 1px 4px;
      border: 1px solid rgba(255, 255, 255, .16);
      border-radius: 3px;
      font: 10px/1.2 Consolas, monospace;
      color: #8296aa;
    }
    @media (max-width: 620px) {
      .hud .bar { width: 64px; }
      .hud .skip kbd { display: none; }
    }
    @media (prefers-reduced-motion: reduce) {
      .stage, .stage.in, .stage.out, .hud, .cursor, .tag { transition: none !important; }
    }
  `;

  // --------------------------------------------------------------------- run

  class Run {
    constructor() {
      this.t0 = performance.now();
      this.finished = false;
      this.started = false;
      this.phase = "idle";
      this.completed = 0;
      this.worked = 0;
      this.tasks = [];
      this.active = new Set();
      this.cursors = [];
      this.lastNow = 0;
      this.baseFont = "";
      this.donePromise = new Promise((resolve) => { this.doneResolve = resolve; });
    }

    async start() {
      this.watchdog = setTimeout(() => this.finish(), 30000);
      this.sx = window.scrollX;
      this.sy = window.scrollY;
      this.bindScrollLock();

      try {
        await Promise.race([document.fonts.ready, sleep(900)]);
      } catch (_) {}

      if (this.finished) return;
      if (document.visibilityState === "hidden") {
        this.finish();
        return;
      }

      this.collect();
      if (!this.tasks.length) {
        this.finish();
        return;
      }

      this.buildStage();
      await nextFrame();
      await nextFrame();
      if (this.finished) return;

      this.stage.classList.add("in");
      this.hud.classList.add("on");
      this.updateHud(true);

      this.lastNow = performance.now();
      this.introT0 = this.lastNow;
      this.phase = "intro";
      this.raf = requestAnimationFrame(this.frame);

      // 给遮罩淡入和多光标集合留出时间
      await sleep(820 / cfg.speed);
      if (this.finished) return;

      this.phase = "work";
      for (const c of this.cursors) {
        c.orbit = null;
        if (!c.busy) {
          c.tx = null;
          c.ty = null;
          c.idle = {
            x: c.x,
            y: c.y,
            rx: rand(16, 52),
            ry: rand(9, 30),
            ph: rand(0, Math.PI * 2),
          };
        }
      }
      this.started = true;
      this.pump();

      await this.donePromise;
      if (this.finished) return;

      this.showDone();
      await sleep(600);
      if (this.finished) return;
      this.finish();
    }

    // ------------------------------------------------------------- scroll lock

    bindScrollLock() {
      this.onWheel = (e) => {
        if (!this.finished) {
          e.preventDefault();
          e.stopPropagation();
        }
      };
      this.onTouch = (e) => {
        if (!this.finished) e.preventDefault();
      };
      this.onKey = (e) => {
        if (this.finished) return;
        if (e.key === "Escape") {
          e.preventDefault();
          this.finish();
          return;
        }
        if ([" ", "PageDown", "PageUp", "Home", "End", "ArrowDown", "ArrowUp"].includes(e.key)) {
          e.preventDefault();
        }
      };
      this.onScroll = () => {
        if (!this.finished && (window.scrollX !== this.sx || window.scrollY !== this.sy)) {
          window.scrollTo(this.sx, this.sy);
        }
      };
      this.onResize = () => {
        if (!this.finished) this.finish();
      };
      this.onVisibility = () => {
        if (document.visibilityState === "hidden" && !this.finished) this.finish();
      };

      window.addEventListener("wheel", this.onWheel, { capture: true, passive: false });
      window.addEventListener("touchmove", this.onTouch, { capture: true, passive: false });
      window.addEventListener("keydown", this.onKey, true);
      window.addEventListener("scroll", this.onScroll, { passive: true });
      window.addEventListener("resize", this.onResize);
      document.addEventListener("visibilitychange", this.onVisibility);
    }

    unbindScrollLock() {
      window.removeEventListener("wheel", this.onWheel, { capture: true });
      window.removeEventListener("touchmove", this.onTouch, { capture: true });
      window.removeEventListener("keydown", this.onKey, true);
      window.removeEventListener("scroll", this.onScroll);
      window.removeEventListener("resize", this.onResize);
      document.removeEventListener("visibilitychange", this.onVisibility);
    }

    // ---------------------------------------------------------------- collect

    collect() {
      const scope = document.querySelector("main.wrap") || document.body;
      const els = Array.from(scope.querySelectorAll("*"));
      const boxMap = new Map();
      const items = [];
      const vw = document.documentElement.clientWidth || innerWidth;
      const vh = document.documentElement.clientHeight || innerHeight;
      this.w = vw;
      this.h = vh;

      const visible = (r, cs) => {
        if (!r || r.width < 1 || r.height < 1) return false;
        if (cs.display === "none" || cs.visibility === "hidden" || parseFloat(cs.opacity) < 0.04) return false;
        return r.bottom > 0 && r.top < vh && r.right > 0 && r.left < vw;
      };

      // 盒子、分隔线、图片
      for (let i = 0; i < els.length; i++) {
        const el = els[i];
        const tag = el.tagName;
        if (tag === "SCRIPT" || tag === "STYLE" || tag === "NOSCRIPT" || tag === "TEMPLATE") continue;
        if (el.closest("[data-swarm-skip]")) continue;

        const cs = getComputedStyle(el);
        if (!visible(el.getBoundingClientRect(), cs)) continue;

        const r = el.getBoundingClientRect();
        const rect = { x: r.left, y: r.top, w: r.width, h: r.height };
        const fill = parseColor(cs.backgroundColor);
        const bw = {
          t: parseFloat(cs.borderTopWidth) || 0,
          r: parseFloat(cs.borderRightWidth) || 0,
          b: parseFloat(cs.borderBottomWidth) || 0,
          l: parseFloat(cs.borderLeftWidth) || 0,
        };
        const bc = parseColor(cs.borderTopColor);
        const anyBorder = bw.t + bw.r + bw.b + bw.l > 0 && bc.a > 0.02;
        const uniform = anyBorder && bw.t > 0 && bw.r > 0 && bw.b > 0 && bw.l > 0;
        const topOnly = anyBorder && bw.t > 0 && bw.r === 0 && bw.b === 0 && bw.l === 0;
        const hasFill = fill.a > 0.02;
        const radius = radiiOf(cs);

        let item = null;
        if (tag === "IMG") {
          item = {
            kind: "image",
            el,
            rect,
            order: i * 10 + 1,
            fill: hasFill ? fill : null,
            border: uniform ? { w: bw.t, color: bc } : null,
            radius,
          };
        } else if (hasFill || uniform) {
          item = {
            kind: "box",
            el,
            rect,
            order: i * 10 + 1,
            fill: hasFill ? fill : null,
            border: uniform ? { w: bw.t, color: bc } : null,
            radius,
          };
        } else if (topOnly) {
          item = {
            kind: "rule",
            el,
            rect,
            order: i * 10 + 1,
            color: bc,
            width: bw.t,
            y: rect.y + bw.t / 2,
          };
        }

        if (item) {
          items.push(item);
          if (item.kind === "box") boxMap.set(el, item);
        }
      }

      // 文本块：每个直接含文字的元素都是一个块；嵌套的直接含文字元素
      // 会被分到自己的块里，避免重复。
      for (let i = 0; i < els.length; i++) {
        const el = els[i];
        const tag = el.tagName;
        if (tag === "SCRIPT" || tag === "STYLE" || tag === "NOSCRIPT" || tag === "TEMPLATE") continue;
        if (!hasDirectText(el)) continue;
        if (el.closest("[data-swarm-skip]")) continue;

        const cs = getComputedStyle(el);
        if (cs.display === "none" || cs.visibility === "hidden" || parseFloat(cs.opacity) < 0.04) continue;
        if (!visible(el.getBoundingClientRect(), cs)) continue;

        const chars = this.collectChars(el, cs);
        if (!chars.length) continue;
        const bounds = {
          x: Math.min(...chars.map((c) => c.x)),
          y: Math.min(...chars.map((c) => c.y - c.h / 2)),
          w: 0,
          h: 0,
        };
        bounds.w = Math.max(...chars.map((c) => c.x + c.w)) - bounds.x;
        bounds.h = Math.max(...chars.map((c) => c.y + c.h / 2)) - bounds.y;

        const item = {
          kind: "text",
          el,
          chars,
          bounds,
          order: i * 10 + 2,
          fontSize: parseFloat(cs.fontSize) || 15,
        };
        item.parent = this.findParentBox(el, boxMap, true);
        if (tag === "LI" && cs.listStyleType !== "none") {
          const lr = el.getBoundingClientRect();
          const first = chars.find((c) => !c.space) || chars[0];
          if (first) item.bullet = { x: lr.left - 15, y: first.y + 1, color: cs.color };
        }
        items.push(item);
      }

      // 盒子按最近的祖先盒子排序：父级先画完，子级再开始，避免父级
      // 的底色把已经画好的子元素盖住。
      for (const item of items) {
        if (item.kind !== "text") item.parent = this.findParentBox(item.el, boxMap, false);
      }

      items.sort((a, b) => a.order - b.order);
      this.items = items;
      this.tasks = items.map((item) => ({
        item,
        state: "wait",
        deps: [],
        p: 0,
        typing: false,
        caret: null,
        badge: null,
      }));
      items.forEach((item, i) => { item.task = this.tasks[i]; });
      for (const item of items) {
        if (item.parent && item.parent.task && item.parent.task !== item.task) {
          item.task.deps.push(item.parent.task);
        }
        if (item.kind === "text") this.prepareTyping(item);
      }
      this.total = this.tasks.length;
    }

    findParentBox(el, boxMap, includeSelf) {
      let node = includeSelf ? el : el && el.parentElement;
      while (node && node !== document.body) {
        if (boxMap.has(node)) return boxMap.get(node);
        node = node.parentElement;
      }
      return null;
    }

    collectChars(root, rootStyle) {
      const chars = [];
      const vw = this.w;
      const vh = this.h;

      const visit = (node) => {
        for (let child = node.firstChild; child; child = child.nextSibling) {
          if (child.nodeType === 3) {
            const text = child.nodeValue;
            if (!text || !text.trim()) continue;
            const parent = child.parentElement;
            if (!parent) continue;
            const cs = parent === root ? rootStyle : getComputedStyle(parent);
            if (cs.display === "none" || cs.visibility === "hidden") continue;
            const font = fontOf(cs);
            const color = cs.color;
            let offset = 0;
            for (const ch of Array.from(text)) {
              const start = offset;
              offset += ch.length;
              const range = document.createRange();
              range.setStart(child, start);
              range.setEnd(child, offset);
              const r = range.getBoundingClientRect();
              if (!r || (!r.width && !r.height)) continue;
              if (r.bottom <= 0 || r.top >= vh || r.right <= 0 || r.left >= vw) continue;
              chars.push({
                ch,
                x: r.left,
                // canvas 的 middle 基线和 DOM 行盒中心差约 1px
                y: r.top + r.height / 2 + 1,
                w: r.width,
                h: r.height,
                font,
                color,
                space: !ch.trim(),
              });
            }
          } else if (child.nodeType === 1) {
            const tag = child.tagName;
            if (tag === "SCRIPT" || tag === "STYLE" || tag === "NOSCRIPT" || tag === "TEMPLATE") continue;
            if (child.hasAttribute && child.hasAttribute("data-swarm-skip")) continue;
            if (hasDirectText(child)) continue;
            visit(child);
          }
        }
      };

      visit(root);
      return chars;
    }

    prepareTyping(item) {
      const visible = item.chars.filter((c) => !c.space).length;
      let target = clamp(150 + visible * 11, 240, 1100);
      if (item.fontSize >= 20) target *= 1.15;
      const weights = item.chars.map((c) => {
        if (c.space) return 0.32;
        return /[，。；：！？、,.;:!?]/.test(c.ch) ? 1.8 : 1;
      });
      const sum = weights.reduce((a, b) => a + b, 0) || 1;
      item.chars.forEach((c, i) => { c.delay = (target * weights[i]) / sum; });
    }

    // ----------------------------------------------------------------- stage

    buildStage() {
      const host = document.createElement("div");
      host.id = "swarm-intro-host";
      host.setAttribute("aria-hidden", "true");
      host.style.cssText =
        "position:fixed;inset:0;display:block;width:100%;height:100%;z-index:2147483000;pointer-events:auto;overflow:hidden;isolation:isolate;";

      const shadow = host.attachShadow({ mode: "open" });
      const style = document.createElement("style");
      style.textContent = CSS;

      const stage = document.createElement("div");
      stage.className = "stage";

      const bg = cssVar("--bg", "#0f141c");
      // 页面本身的 body::before 渐变被不透明的 body 背景盖住了，实际
      // 看到的是纯 --bg；这里保持一致，淡出时不会有色差。
      stage.style.background = bg;

      // 底层画已完成的元素，上层只画进行中的预览和光标；这样每帧
      // 不用把整张底图重新拷一遍，4K 下也轻很多。
      const baseCanvas = document.createElement("canvas");
      baseCanvas.className = "base";
      const canvas = document.createElement("canvas");
      canvas.className = "scene";
      const cursorLayer = document.createElement("div");
      cursorLayer.className = "cursors";

      const hud = document.createElement("div");
      hud.className = "hud";
      hud.innerHTML =
        '<span class="live"></span><span class="txt"></span>' +
        '<span class="bar"><i></i></span>' +
        '<button class="skip" type="button">跳过<kbd>Esc</kbd></button>';
      this.hud = hud;
      this.hudText = hud.querySelector(".txt");
      this.hudBar = hud.querySelector(".bar i");
      hud.querySelector(".skip").addEventListener("click", (e) => {
        e.stopPropagation();
        this.finish();
      });

      stage.appendChild(baseCanvas);
      stage.appendChild(canvas);
      stage.appendChild(cursorLayer);
      stage.appendChild(hud);
      shadow.appendChild(style);
      shadow.appendChild(stage);
      document.documentElement.appendChild(host);

      this.host = host;
      this.stage = stage;
      this.canvas = canvas;
      this.cursorLayer = cursorLayer;

      // 用 clientWidth 而不是 innerWidth：Windows 的经典滚动条会占 15px，
      // 用 innerWidth 会让画布被横向压扁，淡出时和真实页面对不上。
      const w = stage.clientWidth || document.documentElement.clientWidth || innerWidth;
      const h = stage.clientHeight || document.documentElement.clientHeight || innerHeight;
      this.w = w;
      this.h = h;
      const dpr = Math.min(
        window.devicePixelRatio || 1,
        2,
        4096 / Math.max(1, w),
        4096 / Math.max(1, h),
      );
      this.dpr = dpr;

      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      const ctx = canvas.getContext("2d");
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.textAlign = "left";
      ctx.textBaseline = "middle";
      this.ctx = ctx;

      baseCanvas.width = Math.round(w * dpr);
      baseCanvas.height = Math.round(h * dpr);
      const bctx = baseCanvas.getContext("2d");
      bctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      bctx.textAlign = "left";
      bctx.textBaseline = "middle";
      this.baseCtx = bctx;

      // 开场时先留一个半透明的站点图标作为“种子”，光标群围着它集合；
      // 后面的图片任务会把它补全。
      const seed = this.items.find(
        (it) => it.kind === "image" && it.el.classList && it.el.classList.contains("logo"),
      );
      if (seed && seed.el.complete && seed.el.naturalWidth) {
        const box = this.imageBox(seed);
        paintBox(bctx, seed, 0.55);
        bctx.globalAlpha = 0.55;
        bctx.drawImage(seed.el, box.x, box.y, box.w, box.h);
        bctx.globalAlpha = 1;
        this.seedLogo = seed;
      }

      this.makeCursors();
    }

    makeCursors() {
      const seed = this.seedLogo;
      const ar = seed ? seed.rect : null;
      const anchor = ar ? null : document.querySelector(".logo") || document.querySelector(".hero") || document.querySelector("h1");
      const fallback = anchor ? anchor.getBoundingClientRect() : null;
      const cx = clamp(ar ? ar.x + ar.w / 2 : fallback ? fallback.left + fallback.width / 2 : this.w / 2, 70, this.w - 70);
      const cy = clamp(ar ? ar.y + ar.h / 2 : fallback ? fallback.top + fallback.height / 2 : this.h * 0.24, 60, this.h * 0.32);
      const maxR = clamp(Math.min(this.w, this.h) * 0.26, 90, 230);

      const count = Math.min(
        cfg.maxCursors,
        Math.max(8, Math.round(this.w / 95)),
      );

      for (let i = 0; i < count; i++) {
        const color = COLORS[i % COLORS.length];
        const name = NAMES[i % NAMES.length];
        const el = document.createElement("div");
        el.className = "cursor";
        el.innerHTML =
          `<svg viewBox="0 0 20 20" aria-hidden="true"><path d="M1 1 16.5 9.4 9.7 11.2 6.2 17.8Z" ` +
          `fill="${color}" stroke="rgba(255,255,255,.92)" stroke-width="1.1" stroke-linejoin="round"/></svg>`;
        const tag = document.createElement("span");
        tag.className = "tag";
        tag.textContent = name;
        tag.style.background = color;
        tag.style.color = textColorFor(color);
        el.appendChild(tag);
        this.cursorLayer.appendChild(el);

        const angle = rand(0, Math.PI * 2);
        const orbit = {
          cx,
          cy,
          a: angle,
          r0: maxR * rand(1.15, 1.85),
          r: rand(38, 100),
          dir: Math.random() < 0.5 ? -1 : 1,
        };
        const c = {
          el,
          tag,
          name,
          color,
          x: cx + Math.cos(angle) * orbit.r0,
          y: cy + Math.sin(angle) * orbit.r0 * 0.62,
          tx: null,
          ty: null,
          busy: false,
          worked: false,
          orbit,
          idle: null,
        };
        el.style.opacity = "1";
        this.cursors.push(c);
      }
    }

    // ------------------------------------------------------------------ frame

    frame = (now) => {
      if (this.finished) return;
      const dt = Math.min(0.05, (now - (this.lastNow || now)) / 1000);
      this.lastNow = now;
      this.updateCursors(now, dt);
      this.render(now);
      this.updateHud();
      this.raf = requestAnimationFrame(this.frame);
    };

    updateCursors(now, dt) {
      const introT = now - this.introT0;
      for (const c of this.cursors) {
        if (this.phase === "intro" && c.orbit) {
          const e = easeOutCubic(clamp(introT / (820 / cfg.speed), 0, 1));
          const radius = lerp(c.orbit.r0, c.orbit.r, e);
          const angle = c.orbit.a + introT * 0.00055 * c.orbit.dir;
          c.x = c.orbit.cx + Math.cos(angle) * radius;
          c.y = c.orbit.cy + Math.sin(angle) * radius * 0.62;
          c.tx = c.x;
          c.ty = c.y;
        } else if (c.busy) {
          if (c.tx != null) {
            const k = 1 - Math.exp(-dt * 26);
            c.x += (c.tx - c.x) * k;
            c.y += (c.ty - c.y) * k;
          }
        } else if (c.idle) {
          const t = now - this.introT0;
          c.x = c.idle.x + Math.cos(t * 0.00075 + c.idle.ph) * c.idle.rx;
          c.y = c.idle.y + Math.sin(t * 0.00055 + c.idle.ph * 1.7) * c.idle.ry;
        }

        c.el.style.transform =
          `translate3d(${(c.x - 1).toFixed(2)}px, ${(c.y - 1).toFixed(2)}px, 0)`;
        c.el.style.zIndex = String(1000 + Math.round(c.y));
        c.tag.style.opacity = c.busy ? "1" : "0";
      }
    }

    render(now) {
      const ctx = this.ctx;
      ctx.clearRect(0, 0, this.w, this.h);

      for (const task of this.active) {
        const item = task.item;
        if (item.kind === "box") {
          // 从左上角向右下角拖出盒子，而不是整块淡入
          const w = Math.max(1, item.rect.w * task.p);
          const h = Math.max(1, item.rect.h * task.p);
          const preview = {
            ...item,
            rect: { x: item.rect.x, y: item.rect.y, w, h },
            radius: item.radius.map((v) => Math.min(v, w / 2, h / 2)),
          };
          paintBox(ctx, preview, 1);
          if (task.badge && w > 60) this.drawBadge(ctx, task.badge);
        } else if (item.kind === "rule") {
          const x2 = lerp(item.rect.x, item.rect.x + item.rect.w, task.p);
          paintRule(ctx, item, item.rect.x, x2);
        } else if (item.kind === "image") {
          this.drawImageTask(ctx, task);
        } else if (item.kind === "text") {
          this.drawTextTask(ctx, task, now);
        }
      }
    }

    drawBadge(ctx, badge) {
      const text = badge.text;
      ctx.save();
      ctx.font = '500 10px Consolas, "Cascadia Mono", monospace';
      const tw = ctx.measureText(text).width;
      const x = clamp(badge.x, 8, this.w - tw - 24);
      const y = clamp(badge.y, 8, this.h - 26);
      ctx.fillStyle = "rgba(12, 17, 24, .92)";
      roundRect(ctx, x, y, tw + 14, 19, [4, 4, 4, 4]);
      ctx.fill();
      ctx.strokeStyle = "rgba(255, 255, 255, .12)";
      ctx.lineWidth = 1;
      roundRect(ctx, x + 0.5, y + 0.5, tw + 13, 18, [4, 4, 4, 4]);
      ctx.stroke();
      ctx.fillStyle = "#dbe6f0";
      ctx.fillText(text, x + 7, y + 10);
      ctx.restore();
    }

    drawTextTask(ctx, task, now) {
      const item = task.item;
      if (task.typing) {
        const b = item.bounds;
        ctx.save();
        ctx.setLineDash([3, 4]);
        ctx.strokeStyle = "rgba(229, 236, 243, .16)";
        ctx.lineWidth = 1;
        ctx.strokeRect(b.x - 3, b.y - 3, b.w + 6, b.h + 6);
        ctx.restore();
      }
      if (task.caret) {
        const blink = Math.floor(now / 480) % 2 === 0;
        if (blink) {
          const c = this.cursors.find((x) => x.task === task);
          const h = Math.min(task.caret.h - 5, Math.max(10, task.caret.h * 0.78));
          ctx.fillStyle = withAlpha(c ? c.color : "#63dab7", 0.85);
          ctx.fillRect(task.caret.x, task.caret.y - h / 2, 1.6, h);
        }
      }
    }

    drawImageTask(ctx, task) {
      const item = task.item;
      paintBox(ctx, item, task.p);
      if (!item.el.complete || !item.el.naturalWidth) return;
      const box = this.imageBox(item);
      if (box.w < 1 || box.h < 1) return;
      const p = task.p;
      ctx.save();
      ctx.beginPath();
      ctx.rect(box.x, box.y, box.w, box.h * p);
      ctx.clip();
      ctx.drawImage(item.el, box.x, box.y, box.w, box.h);
      ctx.restore();
    }

    imageBox(item) {
      const el = item.el;
      const cs = getComputedStyle(el);
      const bl = parseFloat(cs.borderLeftWidth) || 0;
      const bt = parseFloat(cs.borderTopWidth) || 0;
      const pl = parseFloat(cs.paddingLeft) || 0;
      const pt = parseFloat(cs.paddingTop) || 0;
      const pr = parseFloat(cs.paddingRight) || 0;
      const pb = parseFloat(cs.paddingBottom) || 0;
      return {
        x: item.rect.x + bl + pl,
        y: item.rect.y + bt + pt,
        w: Math.max(1, el.clientWidth - pl - pr),
        h: Math.max(1, el.clientHeight - pt - pb),
      };
    }

    // ------------------------------------------------------------------ tasks

    pump() {
      if (this.finished || !this.started) return;

      while (this.active.size < cfg.maxActive) {
        const task = this.tasks.find(
          (t) => t.state === "wait" && t.deps.every((d) => d.state === "done"),
        );
        if (!task) break;
        const cursor = this.cursors.find((c) => !c.busy);
        if (!cursor) break;
        this.runTask(cursor, task);
      }

      if (this.tasks.every((t) => t.state === "done")) {
        this.doneResolve();
      }
    }

    async runTask(cursor, task) {
      task.state = "active";
      this.active.add(task);
      cursor.busy = true;
      cursor.idle = null;
      cursor.tx = null;
      cursor.ty = null;
      cursor.task = task;
      if (!cursor.worked) {
        cursor.worked = true;
        this.worked++;
      }

      try {
        if (task.item.kind === "box") await this.animateBox(cursor, task);
        else if (task.item.kind === "text") await this.animateText(cursor, task);
        else if (task.item.kind === "rule") await this.animateRule(cursor, task);
        else if (task.item.kind === "image") await this.animateImage(cursor, task);
      } catch (err) {
        if (!this.finished) console.error("[swarm-intro] task failed", err);
      }

      task.state = "done";
      task.caret = null;
      task.badge = null;
      this.active.delete(task);
      this.completed++;
      cursor.busy = false;
      cursor.task = null;
      cursor.tx = null;
      cursor.ty = null;
      if (!this.finished) {
        cursor.idle = {
          x: cursor.x,
          y: cursor.y,
          rx: rand(14, 44),
          ry: rand(8, 26),
          ph: rand(0, Math.PI * 2),
        };
        this.pump();
      }
    }

    async animateBox(cursor, task) {
      const item = task.item;
      const r = item.rect;
      await this.moveCursor(cursor, r.x + 1, r.y + 1, 170, 18);
      const dur = clamp(170 + Math.max(r.w, r.h) * 0.3, 200, 540) / cfg.speed;
      await this.tween(dur, (e) => {
        task.p = e;
        const w = Math.max(1, r.w * e);
        const h = Math.max(1, r.h * e);
        cursor.x = r.x + w;
        cursor.y = r.y + h;
        task.badge =
          r.w >= 80 && r.h >= 24
            ? { text: `${Math.round(r.w)} × ${Math.round(r.h)}`, x: cursor.x + 12, y: cursor.y - 30 }
            : null;
      });
      task.p = 1;
      task.badge = null;
      paintBox(this.baseCtx, item, 1);
    }

    async animateText(cursor, task) {
      const item = task.item;
      const first = item.chars[0];
      await this.moveCursor(cursor, first.x - 1, first.y, 150, 12);
      task.typing = true;
      task.caret = { x: first.x, y: first.y, h: first.h };

      if (item.bullet) {
        const bctx = this.baseCtx;
        bctx.fillStyle = item.bullet.color;
        bctx.beginPath();
        bctx.arc(item.bullet.x, item.bullet.y, 3, 0, Math.PI * 2);
        bctx.fill();
      }

      for (const ch of item.chars) {
        if (this.finished) return;
        if (!ch.space) this.paintChar(ch);
        cursor.tx = ch.x + ch.w + 0.5;
        cursor.ty = ch.y;
        task.caret = { x: ch.x + ch.w + 0.5, y: ch.y, h: ch.h };
        await sleep(ch.delay / cfg.speed);
      }
      task.caret = null;
      task.typing = false;
    }

    paintChar(ch) {
      const bctx = this.baseCtx;
      if (this.baseFont !== ch.font) {
        bctx.font = ch.font;
        this.baseFont = ch.font;
      }
      // 其它任务可能刚画过盒子，会改掉 baseCtx 的 fillStyle；这里不能再
      // 依赖颜色缓存，否则会有零散字符被画成底色。
      bctx.fillStyle = ch.color;
      bctx.globalAlpha = 1;
      bctx.fillText(ch.ch, ch.x, ch.y);
    }

    async animateRule(cursor, task) {
      const item = task.item;
      const x1 = item.rect.x;
      const x2 = item.rect.x + item.rect.w;
      await this.moveCursor(cursor, x1, item.y, 150, 8);
      const dur = clamp(item.rect.w * 0.22, 200, 420) / cfg.speed;
      await this.tween(dur, (e) => {
        task.p = e;
        cursor.x = lerp(x1, x2, e);
        cursor.y = item.y;
      });
      task.p = 1;
      paintRule(this.baseCtx, item, x1, x2);
    }

    async animateImage(cursor, task) {
      const item = task.item;
      const box = this.imageBox(item);
      await this.moveCursor(cursor, box.x + 1, box.y + 1, 140, 10);
      const dur = clamp(220 + box.h * 0.7, 260, 460) / cfg.speed;
      await this.tween(dur, (e) => {
        task.p = e;
        cursor.x = lerp(box.x, box.x + box.w, e);
        cursor.y = box.y + box.h * e;
      });
      task.p = 1;
      const bctx = this.baseCtx;
      paintBox(bctx, item, 1);
      if (item.el.complete && item.el.naturalWidth) {
        bctx.drawImage(item.el, box.x, box.y, box.w, box.h);
        if (item.border) {
          const inset = item.border.w / 2;
          bctx.strokeStyle = rgba(item.border.color, 1);
          bctx.lineWidth = item.border.w;
          bctx.globalAlpha = item.border.color.a || 1;
          roundRect(
            bctx,
            item.rect.x + inset,
            item.rect.y + inset,
            item.rect.w - inset * 2,
            item.rect.h - inset * 2,
            item.radius.map((v) => Math.max(0, v - inset)),
          );
          bctx.stroke();
          bctx.globalAlpha = 1;
        }
      }
    }

    // ----------------------------------------------------------------- motion

    moveCursor(cursor, x, y, dur, arc) {
      cursor.idle = null;
      cursor.orbit = null;
      cursor.tx = null;
      cursor.ty = null;
      const x0 = cursor.x;
      const y0 = cursor.y;
      const dist = Math.hypot(x - x0, y - y0) || 1;
      const ms = (dur || clamp(dist * 0.55, 90, 300)) / cfg.speed;
      return this.tween(ms, (e) => {
        let nx = x0 + (x - x0) * e;
        let ny = y0 + (y - y0) * e;
        if (arc) {
          const s = Math.sin(Math.PI * e) * arc;
          nx += (-(y - y0) / dist) * s;
          ny += ((x - x0) / dist) * s;
        }
        cursor.x = nx;
        cursor.y = ny;
      });
    }

    tween(duration, step) {
      return new Promise((resolve) => {
        const t0 = performance.now();
        const dur = Math.max(1, duration);
        const tick = (now) => {
          if (this.finished) {
            resolve();
            return;
          }
          const p = clamp((now - t0) / dur, 0, 1);
          step(easeInOutCubic(p));
          if (p < 1) requestAnimationFrame(tick);
          else resolve();
        };
        requestAnimationFrame(tick);
      });
    }

    // -------------------------------------------------------------------- hud

    updateHud(force) {
      if (!this.hud || this.finished) return;
      const total = this.total || 1;
      const pct = Math.min(100, (this.completed / total) * 100);
      if (pct !== this.lastPct) {
        this.hudBar.style.width = `${pct}%`;
        this.lastPct = pct;
      }
      let text;
      if (this.phase === "intro") text = `${this.cursors.length} 位协作者已就位`;
      else text = `${this.worked} 位协作者正在重建页面`;
      if (force || text !== this.lastHudText) {
        this.hudText.textContent = text;
        this.lastHudText = text;
      }
    }

    showDone() {
      if (!this.hud) return;
      const secs = ((performance.now() - this.t0) / 1000).toFixed(1);
      this.hudText.textContent = `重建完成 · ${this.worked} 位协作者 · ${secs} 秒`;
      this.hudBar.style.width = "100%";
      const skip = this.hud.querySelector(".skip");
      if (skip) skip.remove();
    }

    // ----------------------------------------------------------------- finish

    finish() {
      if (this.finished) return;
      this.finished = true;
      this.doneResolve();
      clearTimeout(this.watchdog);
      cancelAnimationFrame(this.raf);
      this.unbindScrollLock();

      const host = this.host;
      if (host) {
        if (this.stage) {
          this.stage.classList.remove("in");
          this.stage.classList.add("out");
        }
        if (this.hud) this.hud.classList.remove("on");
        host.style.pointerEvents = "none";
        setTimeout(() => {
          try { host.remove(); } catch (_) {}
        }, 700);
      }
      this.host = null;
    }
  }

  // --------------------------------------------------------------------- api

  let run = null;

  function launch(auto) {
    if (run) return false;
    if (auto && !AUTO_OK) return false;
    try {
      if (cfg.oncePerSession) sessionStorage.setItem(cfg.sessionKey, "1");
    } catch (_) {}
    run = new Run();
    run
      .start()
      .catch((err) => {
        console.error("[swarm-intro]", err);
        if (run) run.finish();
      })
      .finally(() => { run = null; });
    return true;
  }

  window.SwarmIntro = {
    play() {
      return launch(false);
    },
    skip() {
      if (run) run.finish();
    },
    get running() {
      return !!run;
    },
  };

  if (AUTO_OK && (FORCED || cfg.auto !== false)) {
    const boot = () => setTimeout(() => launch(true), 120);
    if (document.readyState === "complete") boot();
    else window.addEventListener("load", boot, { once: true });
  }
})();
