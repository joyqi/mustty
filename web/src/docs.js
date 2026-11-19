import "./style.css";
import { followTheme } from "./theme.js";

followTheme(document.getElementById("theme-toggle"));

// Marks the table-of-contents link of the section being read: the last heading above a line 30% down
// the viewport. The observer fires whenever a heading crosses that line.
const links = [...document.querySelectorAll("#toc a")];
const headings = links.map((a) => document.getElementById(a.hash.slice(1)));
const mark = () => {
  const line = innerHeight * 0.3;
  const current = headings.findLast((h) => h.getBoundingClientRect().top <= line);
  links.forEach((a, i) => a.toggleAttribute("aria-current", headings[i] === current));
};
const observer = new IntersectionObserver(mark, { rootMargin: "0px 0px -70% 0px" });
for (const h of headings) observer.observe(h);
