// Light/dark theme for the page and the terminal. mustty only emits the 16 ANSI colors (SGR 30–37/90–97:
// the code color, the saved dot, VIS), never RGB, so these palettes decide every color it draws.
// GitHub's dark and light terminal colors.
export const THEMES = {
  dark: {
    background: "#0d1117", foreground: "#e6edf3", cursor: "#e6edf3", cursorAccent: "#0d1117",
    selectionBackground: "#388bfd66",
    black: "#484f58", red: "#ff7b72", green: "#3fb950", yellow: "#d29922",
    blue: "#58a6ff", magenta: "#bc8cff", cyan: "#39c5cf", white: "#b1bac4",
    brightBlack: "#6e7681", brightRed: "#ffa198", brightGreen: "#56d364", brightYellow: "#e3b341",
    brightBlue: "#79c0ff", brightMagenta: "#d2a8ff", brightCyan: "#56d4dd", brightWhite: "#ffffff",
  },
  light: {
    background: "#ffffff", foreground: "#1f2328", cursor: "#1f2328", cursorAccent: "#ffffff",
    selectionBackground: "#0969da33",
    black: "#24292f", red: "#cf222e", green: "#1a7f37", yellow: "#9a6700",
    blue: "#0969da", magenta: "#8250df", cyan: "#1b7c83", white: "#6e7781",
    brightBlack: "#57606a", brightRed: "#a40e26", brightGreen: "#116329", brightYellow: "#7d4e00",
    brightBlue: "#218bff", brightMagenta: "#a475f9", brightCyan: "#3192aa", brightWhite: "#8c959f",
  },
};

// A manual choice (one localStorage key) wins over the system setting until it is changed again.
// The inline script in index.html applies the same rule before first paint.
const KEY = "mustty-theme";
const stored = () => { try { return localStorage.getItem(KEY); } catch { return null; } };
const dark = matchMedia("(prefers-color-scheme: dark)");
const pick = () => {
  const s = stored();
  return s === "light" || s === "dark" ? s : dark.matches ? "dark" : "light";
};

const listeners = [];
const root = document.documentElement;

export const currentTheme = () => THEMES[root.dataset.theme === "light" ? "light" : "dark"];

// Calls `listener` with the new xterm theme whenever the page theme changes
export const onThemeChange = (listener) => listeners.push(listener);

// Keeps the page theme in sync with the system setting (unless a manual choice is stored) and lets
// `toggle` switch it by hand
export function followTheme(toggle) {
  const set = (next) => {
    if (next === root.dataset.theme) return;
    root.dataset.theme = next;
    for (const listener of listeners) listener(THEMES[next]);
  };
  set(pick());
  dark.addEventListener("change", () => set(pick()));
  toggle.addEventListener("click", () => {
    const next = root.dataset.theme === "dark" ? "light" : "dark";
    try { localStorage.setItem(KEY, next); } catch {}
    set(next);
  });
}
