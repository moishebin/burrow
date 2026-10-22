// Inline SVG icons (static, trusted markup). Stroke icons in the style of Lucide (ISC license).
"use strict";
const ICONS = (() => {
  const s = (body, size = 16) =>
    `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`;
  return {
    sidebar: s('<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M9 4v16"/>'),
    back: s('<path d="M19 12H5"/><path d="m12 19-7-7 7-7"/>'),
    forward: s('<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>'),
    reload: s('<path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><path d="M21 3v5h-5"/>'),
    stop: s('<path d="M18 6 6 18"/><path d="m6 6 12 12"/>'),
    lock: s('<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>', 12),
    folder: s('<path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.7-.9l-.8-1.2A2 2 0 0 0 7.9 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>'),
    folderOpen: s('<path d="m6 14 1.5-2.9A2 2 0 0 1 9.2 10H20a2 2 0 0 1 1.9 2.5l-1.5 6A2 2 0 0 1 18.5 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.7.9l.8 1.2a2 2 0 0 0 1.7.9H18a2 2 0 0 1 2 2v2"/>'),
    close: s('<path d="M18 6 6 18"/><path d="m6 6 12 12"/>', 14),
    minus: s('<path d="M5 12h14"/>', 14),
    plus: s('<path d="M12 5v14"/><path d="M5 12h14"/>'),
    dots: s('<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>'),
    download: s('<path d="M12 15V3"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/>'),
    globe: s('<circle cx="12" cy="12" r="10"/><path d="M2 12h20"/><path d="M12 2a15 15 0 0 1 0 20 15 15 0 0 1 0-20"/>'),
    winMin: s('<path d="M6 12h12"/>', 14),
    winMax: s('<rect x="6" y="6" width="12" height="12" rx="1.5"/>', 14),
    winRestore: s('<rect x="5" y="9" width="10" height="10" rx="1.5"/><path d="M9 9V6.5A1.5 1.5 0 0 1 10.5 5h7A1.5 1.5 0 0 1 19 6.5v7a1.5 1.5 0 0 1-1.5 1.5H15"/>', 14),
    winClose: s('<path d="M7 7l10 10"/><path d="M17 7 7 17"/>', 14),
    home: s('<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>', 13),
  };
})();
