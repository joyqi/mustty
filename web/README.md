mustty in the browser, through xterm.js. Needs [trunk](https://trunkrs.dev) and the `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`).
Run locally: `trunk serve` in this directory, then open http://localhost:8080.
Build for hosting: `trunk build --release` writes the site to `web/dist/`.
