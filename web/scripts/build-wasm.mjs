// Builds crates/mustty-web into src/pkg. wasm-pack comes from npm as a prebuilt binary and downloads
// the wasm-bindgen CLI matching Cargo.lock, so Rust is the only thing a machine needs beforehand; on
// Cloudflare Pages (CF_PAGES=1), whose image has no Rust, this installs it first. Safe to rerun.
//
//   node scripts/build-wasm.mjs          release build, wasm-opt -Oz
//   node scripts/build-wasm.mjs --dev    debug build, faster, for `pnpm dev`
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, dirname, join } from "node:path";

const TARGET = "wasm32-unknown-unknown";
const run = (cmd, args, env = process.env) => execFileSync(cmd, args, { stdio: "inherit", env });
const output = (cmd, args) => {
  const r = spawnSync(cmd, args, { encoding: "utf8" });
  return r.status === 0 ? r.stdout.trim() : null;
};

const cargoBin = join(process.env.CARGO_HOME ?? join(homedir(), ".cargo"), "bin");
const findRustup = () =>
  output("rustup", ["--version"]) !== null ? "rustup"
  : existsSync(join(cargoBin, "rustup")) ? join(cargoBin, "rustup")
  : null;

let rustup = findRustup();
if (!rustup && output("cargo", ["--version"]) === null) {
  if (!process.env.CF_PAGES) {
    console.error("Rust is needed to build the wasm: install it with rustup (https://rustup.rs)");
    process.exit(1);
  }
  run("sh", ["-c", `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --target ${TARGET}`]);
  rustup = join(cargoBin, "rustup");
}

let env = process.env;
if (rustup) {
  run(rustup, ["target", "add", TARGET]);
  // rustup's toolchain first: a cargo/rustc earlier on PATH (Homebrew's, say) has no wasm32 std.
  // ~/.cargo/bin goes last so the wasm-pack from node_modules/.bin wins over a global one
  const toolchainBin = dirname(output(rustup, ["which", "cargo"]));
  env = { ...env, PATH: [toolchainBin, env.PATH, cargoBin].join(delimiter) };
}

const profile = process.argv.includes("--dev") ? "--dev" : "--release";
// --out-dir is relative to the crate
run("wasm-pack", ["build", "../crates/mustty-web", profile, "--target", "web", "--out-dir", "../../web/src/pkg", "--no-pack", "--no-typescript"], env);
