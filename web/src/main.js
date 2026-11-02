import "./style.css";
import { followTheme } from "./theme.js";

followTheme(document.getElementById("theme-toggle"));

// Install tabs: each tab carries its command in data-command
const tabs = [...document.querySelectorAll("[role=tab]")];
const command = document.getElementById("install-command");
for (const tab of tabs) {
  tab.addEventListener("click", () => {
    for (const t of tabs) t.setAttribute("aria-selected", String(t === tab));
    command.textContent = tab.dataset.command;
  });
}

const copy = document.getElementById("copy");
const copyLabel = copy.querySelector("span");
let copyTimer;
copy.addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText(command.textContent);
  } catch {
    // No clipboard API (an http:// origin other than localhost): select the command for ⌘C instead
    getSelection().selectAllChildren(command);
    return;
  }
  copyLabel.textContent = "Copied";
  clearTimeout(copyTimer);
  copyTimer = setTimeout(() => { copyLabel.textContent = "Copy"; }, 2000);
});

// The terminal: fetch xterm.js and the wasm only when the demo comes within 200px of the viewport
const demo = document.getElementById("demo");
const host = document.getElementById("terminal");
let focusWhenReady = false;
let term;
const observer = new IntersectionObserver(async (entries) => {
  if (!entries.some((e) => e.isIntersecting)) return;
  observer.disconnect();
  try {
    const { mountTerminal } = await import("./terminal.js");
    term = await mountTerminal(host);
    document.getElementById("loading").remove();
    if (focusWhenReady) term.textarea?.focus({ preventScroll: true });
  } catch (e) {
    document.getElementById("loading").textContent = "mustty could not start in this browser.";
    throw e;
  }
}, { rootMargin: "200px 0px" });
observer.observe(demo);

// "try it in your browser": scroll smoothly to the demo and put the cursor in the terminal. Done here
// rather than by following the #demo link, which would take the focus back to the page
document.getElementById("try").addEventListener("click", (e) => {
  e.preventDefault();
  demo.scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  history.replaceState(null, "", "#demo");
  if (term) term.textarea?.focus({ preventScroll: true });
  else focusWhenReady = true;
});
