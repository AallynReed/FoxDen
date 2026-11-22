// Shows the UI in the chosen language: English text is swapped for its translation wherever it
// appears, including text the scripts set later. Missing strings stay in English.
(function () {
  const LANGS = {
    en: "English",
    "pt-PT": "Português (Portugal)",
    es: "Español",
    de: "Deutsch",
    fr: "Français",
    ru: "Русский",
    "zh-Hans": "简体中文",
    pl: "Polski",
    tr: "Türkçe",
    ja: "日本語",
    ko: "한국어",
    it: "Italiano",
    uk: "Українська",
    "zh-Hant": "繁體中文",
    nl: "Nederlands",
    cs: "Čeština",
  };

  // A Windows or browser language tag to one of ours: any Portuguese gets Portugal's, Chinese splits by script.
  function pick(tag) {
    const t = (tag || "").toLowerCase();
    if (t.startsWith("zh")) return /hant|tw|hk|mo/.test(t) ? "zh-Hant" : "zh-Hans";
    if (t.startsWith("pt")) return "pt-PT";
    const base = t.split("-")[0];
    return Object.keys(LANGS).find((k) => k === base) || null;
  }

  function stored() {
    try {
      return localStorage.getItem("foxden-language") || "auto";
    } catch {
      return "auto";
    }
  }

  const choice = stored();
  const lang = choice === "auto" ? (navigator.languages || [navigator.language]).map(pick).find(Boolean) || "en" : LANGS[choice] ? choice : "en";
  document.documentElement.lang = lang;

  let table = {};
  if (lang !== "en") {
    try {
      const req = new XMLHttpRequest();
      req.open("GET", "lang/" + lang + ".json", false);
      req.send();
      if (req.status === 200 || req.status === 0) table = JSON.parse(req.responseText);
    } catch {
      table = {};
    }
  }

  const exact = new Map();
  const templates = [];
  for (const [en, out] of Object.entries(table)) {
    if (!out || out === en) continue;
    if (/\{\d+\}/.test(en)) {
      const parts = en.split(/(\{\d+\})/);
      const order = [];
      const source = parts.map((p) => (/^\{\d+\}$/.test(p) ? (order.push(p), "(.+?)") : p.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))).join("");
      templates.push({ re: new RegExp("^" + source + "$", "s"), order, out, literal: en.replace(/\{\d+\}/g, "").length });
    } else {
      exact.set(en, out);
    }
  }
  // The most specific template wins.
  templates.sort((a, b) => b.literal - a.literal);

  const cache = new Map();
  // Text seen without a translation, for finding gaps: __foxI18n.missing.
  const missing = new Set();
  const done = new Set(exact.values());

  function translate(text) {
    if (!text || exact.size + templates.length === 0) return text;
    const trimmed = text.trim();
    if (!trimmed || !/[A-Za-z]/.test(trimmed) || done.has(trimmed)) return text;
    let out = exact.get(trimmed);
    if (out === undefined) {
      if (cache.has(trimmed)) out = cache.get(trimmed);
      else {
        out = null;
        for (const t of templates) {
          const m = trimmed.match(t.re);
          if (!m) continue;
          out = t.out;
          t.order.forEach((slot, i) => {
            const part = m[i + 1];
            out = out.split(slot).join(exact.get(part) || part);
          });
          break;
        }
        if (cache.size > 5000) cache.clear();
        cache.set(trimmed, out);
        if (out) done.add(out);
      }
    }
    if (!out && trimmed.includes(" · ")) {
      const parts = trimmed.split(" · ");
      const each = parts.map((p) => translate(p));
      if (each.some((p, i) => p !== parts[i])) out = each.join(" · ");
    }
    if (!out) {
      missing.add(trimmed);
      return text;
    }
    return text.replace(trimmed, out);
  }

  const ATTRS = ["title", "aria-label", "placeholder", "alt"];
  const SKIP = new Set(["SCRIPT", "STYLE", "TEXTAREA", "CODE", "PRE"]);

  function skipped(el) {
    return !el || SKIP.has(el.nodeName) || el.closest("[translate=no]");
  }

  function textNode(node) {
    if (skipped(node.parentElement)) return;
    const next = translate(node.nodeValue);
    if (next !== node.nodeValue) node.nodeValue = next;
  }

  function element(el) {
    if (skipped(el)) return;
    for (const name of ATTRS) {
      const v = el.getAttribute(name);
      if (v) {
        const next = translate(v);
        if (next !== v) el.setAttribute(name, next);
      }
    }
  }

  function walk(root) {
    if (root.nodeType === Node.TEXT_NODE) return textNode(root);
    if (root.nodeType !== Node.ELEMENT_NODE) return;
    element(root);
    const it = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
    for (let n = it.nextNode(); n; n = it.nextNode()) {
      if (n.nodeType === Node.TEXT_NODE) textNode(n);
      else element(n);
    }
  }

  function start() {
    walk(document.body);
    document.title = translate(document.title);
    new MutationObserver((records) => {
      for (const r of records) {
        if (r.type === "characterData") textNode(r.target);
        else if (r.type === "attributes") element(r.target);
        else for (const n of r.addedNodes) walk(n);
      }
    }).observe(document.body, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ATTRS });
  }

  if (lang !== "en") {
    if (document.body) start();
    else document.addEventListener("DOMContentLoaded", start);
  }

  window.__foxI18n = {
    lang,
    choice,
    languages: LANGS,
    t: translate,
    missing,
    // Saves the choice and shows the UI in it.
    set(next) {
      try {
        localStorage.setItem("foxden-language", next);
      } catch {}
      // A moment for the setting to be saved first.
      if (next !== choice) setTimeout(() => location.reload(), 300);
    },
  };
})();
