// Generates src/code/tables.rs, the code lexer's word tables, from the highlight.js language modules
// pinned in package-lock.json plus the corrections in overrides.toml.
//
//   cd tools/lexer-tables && npm ci && node gen.mjs
//
// A language's keywords are highlight.js's `keyword`, `literal`, `variable.language` and
// `variable.constant` words; its types are the `type` words. A word in both lists stays a type.
// Words highlight.js also files under `built_in` (library functions and classes: SQL's
// current_date, Dart's print) are dropped: only the language's own words are kept, and any
// builtin we do want (C#'s `int`, Python's `str`) is added by name in overrides.toml.
// tests/code_scan.rs checks the hashes in the generated header, so a hand edit to tables.rs or an
// edit to overrides.toml without regenerating fails `cargo test`.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, "../../src/code/tables.rs");
const require = createRequire(import.meta.url);
const hljs = require("highlight.js/lib/core");

// The constant in tables.rs and the highlight.js module it comes from
const LANGS = [
  ["RUST", "rust"],
  ["JAVASCRIPT", "javascript"],
  ["TYPESCRIPT", "typescript"],
  ["PYTHON", "python"],
  ["GO", "go"],
  ["BASH", "bash"],
  ["POWERSHELL", "powershell"],
  ["JSON", "json"],
  ["YAML", "yaml"],
  ["TOML", "ini"],
  ["SQL", "sql"],
  ["HTML", "xml"],
  ["CSS", "css"],
  ["C", "c"],
  ["CPP", "cpp"],
  ["JAVA", "java"],
  ["CSHARP", "csharp"],
  ["PHP", "php"],
  ["KOTLIN", "kotlin"],
  ["LUA", "lua"],
  ["RUBY", "ruby"],
  ["DART", "dart"],
  ["SWIFT", "swift"],
  ["MARKDOWN", "markdown"],
  ["DIFF", "diff"],
  ["XML", "xml"],
  ["SCSS", "scss"],
  ["DOCKERFILE", "dockerfile"],
  ["MAKEFILE", "makefile"],
  ["PROTOBUF", "protobuf"],
];

const KEYWORD_CATEGORIES = ["keyword", "literal", "variable.language", "variable.constant"];

const words = (v) =>
  !v ? [] : typeof v === "string" ? v.split(/\s+/).filter(Boolean) : Array.isArray(v) ? v.flatMap(words) : [];

/// The subset of TOML that overrides.toml uses: `[section]` and `key = ["string", …]`, arrays may
/// span lines, `#` comments
function parseOverrides(text) {
  const result = {};
  let section = null;
  let pending = null;
  for (const raw of text.split("\n")) {
    const line = raw.replace(/#.*$/, "").trim();
    if (pending) {
      pending.text += " " + line;
    } else if (/^\[[\w-]+\]$/.test(line)) {
      section = line.slice(1, -1);
      result[section] = {};
      continue;
    } else if (line) {
      const m = /^([\w-]+)\s*=\s*(.*)$/.exec(line);
      if (!m || !section) throw new Error(`overrides.toml: can't read ${JSON.stringify(raw)}`);
      pending = { key: m[1], text: m[2] };
    }
    if (pending && pending.text.includes("]")) {
      result[section][pending.key] = JSON.parse(pending.text.replace(/,\s*\]/, "]"));
      pending = null;
    }
  }
  return result;
}

const sha256 = (s) => createHash("sha256").update(s).digest("hex");
const overridesText = readFileSync(join(here, "overrides.toml"), "utf8").replace(/\r\n/g, "\n");
const overrides = parseOverrides(overridesText);
const KNOWN = new Set(["add", "remove", "add_types", "remove_types"]);

function table(name, module) {
  const lang = require(`highlight.js/lib/languages/${module}`)(hljs);
  const kw = lang.keywords ?? {};
  const cats = typeof kw === "string" || Array.isArray(kw) ? { keyword: kw } : kw;
  const ci = !!lang.case_insensitive;
  const norm = (w) => (ci ? w.toLowerCase() : w);
  const pick = (names) =>
    names.flatMap((c) => words(cats[c])).map((w) => w.split("|")[0]);
  const builtins = new Set(pick(["built_in"]).map(norm));
  // Only words the lexer can see as one identifier (drops Swift's `#if`), and no builtins
  const valid = (w) => /^-?[A-Za-z_][A-Za-z0-9_]*$/.test(w);
  const keep = (list) => {
    const all = new Set(list.filter(valid).map(norm));
    const own = [...all].filter((w) => !builtins.has(w));
    excluded += all.size - own.length;
    return new Set(own);
  };
  const k = keep(pick(KEYWORD_CATEGORIES));
  const t = keep(pick(["type"]));

  const fix = overrides[name.toLowerCase()] ?? {};
  for (const key of Object.keys(fix)) {
    if (!KNOWN.has(key)) throw new Error(`overrides.toml [${name.toLowerCase()}]: unknown key ${key}`);
  }
  const apply = (set, list, add, what) => {
    for (const w of (list ?? []).map(norm)) {
      // Fail loudly when upstream already made the correction, so overrides.toml stays minimal
      if (add === set.has(w)) {
        throw new Error(`${name}: ${add ? "add" : "remove"} ${what} ${w}: already ${add ? "there" : "gone"}`);
      }
      add ? set.add(w) : set.delete(w);
    }
  };
  apply(k, fix.remove, false, "keyword");
  apply(t, fix.remove_types, false, "type");
  apply(k, fix.add, true, "keyword");
  apply(t, fix.add_types, true, "type");
  for (const w of t) k.delete(w);
  // Rust's str ordering is byte order, as is a plain JS sort of ASCII strings
  const sorted = (s) => [...s].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  return { name, module, keywords: sorted(k), types: sorted(t), ci };
}

/// `&[...]` wrapped at 100 columns, indented by eight spaces
function list(items) {
  if (items.length === 0) return "&[]";
  const lines = [];
  let line = "";
  for (const w of items.map((w) => `"${w}",`)) {
    if (line && 8 + line.length + 1 + w.length > 100) {
      lines.push(line);
      line = w;
    } else {
      line = line ? `${line} ${w}` : w;
    }
  }
  lines.push(line);
  return `&[\n${lines.map((l) => `        ${l}`).join("\n")}\n    ]`;
}

let excluded = 0;
const tables = LANGS.map(([name, module]) => table(name, module));
const body =
  "use super::Table;\n" +
  tables
    .map(
      (t) =>
        `\n// highlight.js: ${t.module}\npub const ${t.name}: Table = Table {\n` +
        `    keywords: ${list(t.keywords)},\n` +
        `    types: ${list(t.types)},\n` +
        `    case_insensitive: ${t.ci},\n};\n`,
    )
    .join("");

const version = require("highlight.js/package.json").version;
const lock = JSON.parse(readFileSync(join(here, "package-lock.json"), "utf8"));
const integrity = lock.packages["node_modules/highlight.js"].integrity;
const header = [
  "// @generated by tools/lexer-tables/gen.mjs. Do not edit: change overrides.toml and regenerate.",
  "// Regenerate: cd tools/lexer-tables && npm ci && node gen.mjs",
  `// Source: highlight.js ${version} (BSD-3-Clause, see LICENSE), ${integrity}`,
  `// overrides.toml sha256: ${sha256(overridesText)}`,
  `// Body sha256 (everything after the blank line below): ${sha256(body)}`,
].join("\n");
writeFileSync(out, `${header}\n\n${body}`);

const total = tables.reduce((n, t) => n + t.keywords.length + t.types.length, 0);
for (const t of tables) console.log(`${t.name.padEnd(11)} ${String(t.keywords.length).padStart(4)} keywords ${String(t.types.length).padStart(3)} types`);
console.log(`${total} words (${excluded} highlight.js builtins left out) -> ${out}`);
