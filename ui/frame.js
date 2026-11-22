// Keeps the web view from acting like a browser: no page menu, reload, print or history keys.
(() => {
  const editable = (t) =>
    t instanceof Element && !!t.closest("textarea, [contenteditable=''], [contenteditable=true], input:not([type]), input[type=text], input[type=search], input[type=number], input[type=email], input[type=url]");

  document.addEventListener("contextmenu", (e) => {
    if (!editable(e.target)) e.preventDefault();
  });

  const blocked = (e) => {
    const key = e.key.toLowerCase();
    const ctrl = e.ctrlKey || e.metaKey;
    if (key === "f5" || key === "browserback" || key === "browserforward" || key === "browserrefresh") return true;
    if (e.altKey && (key === "arrowleft" || key === "arrowright")) return true;
    return ctrl && !e.altKey && ["r", "p", "u", "s"].includes(key);
  };
  // Bubble phase on window, so the app's own Ctrl+S handler has already run.
  window.addEventListener("keydown", (e) => {
    if (blocked(e)) e.preventDefault();
  });

  for (const type of ["mouseup", "auxclick"]) {
    window.addEventListener(type, (e) => {
      if (e.button === 3 || e.button === 4) e.preventDefault();
    });
  }
})();
