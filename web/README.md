# mustty web demo

The mustty landing page, with mustty itself running in it: the Rust crate in `../crates/mustty-web`
compiled to wasm, drawing through xterm.js. A Vite + pnpm project with Tailwind CSS v4 (through
`@tailwindcss/vite`); `dist/` is a static site.

- `index.html` is the page; colors are CSS variables in `src/style.css` (`--bg`, `--text`, …),
  switched by `data-theme` on `<html>`, and `src/theme.js` holds the matching xterm palettes.
- `src/terminal.js` (xterm.js and the wasm) is imported only when the demo comes within 200px of
  the viewport, so the first paint is the HTML, one stylesheet and about 5 KB of script.
- Inter and JetBrains Mono come from Google Fonts without blocking render; metric-matched local
  fallbacks keep the layout identical until they arrive.

## Run locally

Needs Node 20.19+ / 22.12+, pnpm, and Rust via [rustup](https://rustup.rs).

```sh
pnpm install
pnpm dev      # debug wasm build, then the Vite dev server on http://localhost:5173
pnpm build    # release wasm build (wasm-opt -Oz), then vite build into dist/
pnpm preview  # serve dist/
```

`pnpm build:wasm` rebuilds only the wasm into `src/pkg/` (generated, not checked in). Every build
step can be rerun.

## How the wasm is built

`scripts/build-wasm.mjs` runs `wasm-pack build --target web`. wasm-pack is an npm devDependency
(a prebuilt binary, pinned in `package.json`), and it downloads the wasm-bindgen CLI whose version
matches `wasm-bindgen` in `Cargo.lock` (0.2.130 now), so the two can never drift and nothing needs
`cargo install`. Compiling `wasm-bindgen-cli` from source instead takes about 1.5 minutes on a fast
machine, more on a build server.

When rustup is present the script adds the `wasm32-unknown-unknown` target and puts rustup's
toolchain first on PATH (a Homebrew `rustc` has no wasm32 standard library).

## Cloudflare Pages

| Setting | Value |
| --- | --- |
| Root directory | `web` |
| Build command | `pnpm build` |
| Build output directory | `dist` |

Use build image v3 (Node 22, pnpm 10). Nothing needs installing beforehand: the image has no Rust,
so on Cloudflare (`CF_PAGES=1`) the build script installs a minimal rustup toolchain with the wasm32
target first. That adds about 30 seconds; the release build then takes about another 40.
