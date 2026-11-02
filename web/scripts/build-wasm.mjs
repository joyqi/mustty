// Builds crates/mustty-web into src/pkg. wasm-pack comes from npm as a prebuilt binary and downloads
// the wasm-bindgen CLI matching Cargo.lock, so Rust with the wasm32 target is the only thing a machine
// needs beforehand. Safe to rerun. Rust is found in this order, and the path taken is logged:
//
//   1. cargo on PATH whose rustc already has the wasm32 target (GitHub Actions, most setups): used as is
//   2. rustup: its toolchain goes first on PATH, adding the target if it is missing
//   3. Cloudflare Pages (CF_PAGES=1), whose image has no Rust: a minimal rustup toolchain is installed
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
const log = (message) => console.log(`build-wasm: ${message}`);

// Whether `rustc` can build for wasm32: its sysroot has the target's standard library
const hasTarget = (rustc) => {
  const sysroot = output(rustc, ["--print", "sysroot"]);
  return sysroot !== null && existsSync(join(sysroot, "lib", "rustlib", TARGET));
};

const cargoBin = join(process.env.CARGO_HOME ?? join(homedir(), ".cargo"), "bin");
const findRustup = () =>
  output("rustup", ["--version"]) !== null ? "rustup"
  : existsSync(join(cargoBin, "rustup")) ? join(cargoBin, "rustup")
  : null;

let env = process.env;
if (output("cargo", ["--version"]) !== null && hasTarget("rustc")) {
  log(`using ${output("rustc", ["--version"])} from PATH, ${TARGET} installed`);
} else {
  let rustup = findRustup();
  if (rustup) {
    log(`the rustc on PATH has no ${TARGET} target; using rustup's toolchain`);
  } else if (process.env.CF_PAGES) {
    log("no Rust on this Cloudflare Pages image; installing a minimal rustup toolchain");
    run("sh", ["-c", `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --target ${TARGET}`]);
    rustup = join(cargoBin, "rustup");
  } else {
    console.error(`Rust with the ${TARGET} target is needed to build the wasm: install it with rustup (https://rustup.rs)`);
    process.exit(1);
  }
  const rustc = output(rustup, ["which", "rustc"]);
  if (!hasTarget(rustc)) {
    log(`adding the ${TARGET} target`);
    run(rustup, ["target", "add", TARGET]);
  }
  // rustup's toolchain first: a cargo/rustc earlier on PATH (Homebrew's, say) has no wasm32 std.
  // ~/.cargo/bin goes last so the wasm-pack from node_modules/.bin wins over a global one
  env = { ...env, PATH: [dirname(rustc), env.PATH, cargoBin].join(delimiter) };
  log(`using ${output(rustc, ["--version"])} from ${dirname(rustc)}`);
}

const profile = process.argv.includes("--dev") ? "--dev" : "--release";
// --out-dir is relative to the crate
run("wasm-pack", ["build", "../crates/mustty-web", profile, "--target", "web", "--out-dir", "../../web/src/pkg", "--no-pack", "--no-typescript"], env);
