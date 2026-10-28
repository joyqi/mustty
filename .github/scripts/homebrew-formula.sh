#!/usr/bin/env bash
# Prints Formula/mustty.rb for a release.
# Usage: homebrew-formula.sh <tag> <dir with mustty-<target>.tar.gz.sha256 files>
# BASE_URL overrides the download location (default: the GitHub Release).
set -euo pipefail

tag="${1:?usage: homebrew-formula.sh <tag> <sha256-dir>}"
dir="${2:?usage: homebrew-formula.sh <tag> <sha256-dir>}"
base="${BASE_URL:-https://github.com/joyqi/mustty/releases/download/$tag}"

# Prints the url and sha256 lines for one target, indented for an on_* block.
asset() {
  local name="mustty-$1.tar.gz"
  local file="$dir/$name.sha256"
  if [ ! -s "$file" ]; then
    echo "missing checksum file $file" >&2
    exit 1
  fi
  local sum
  sum=$(cut -d' ' -f1 "$file")
  if ! [[ "$sum" =~ ^[0-9a-f]{64}$ ]]; then
    echo "bad sha256 in $file: $sum" >&2
    exit 1
  fi
  printf '      url "%s/%s"\n      sha256 "%s"\n' "$base" "$name" "$sum"
}

# Assigned first so a missing checksum aborts (set -e) before anything is printed
mac_arm=$(asset aarch64-apple-darwin)
mac_intel=$(asset x86_64-apple-darwin)
linux_arm=$(asset aarch64-unknown-linux-musl)
linux_intel=$(asset x86_64-unknown-linux-musl)

cat <<RUBY
class Mustty < Formula
  desc "Distraction-free Markdown writer for the terminal"
  homepage "https://github.com/joyqi/mustty"
  version "${tag#v}"
  license "MIT"

  on_macos do
    on_arm do
$mac_arm
    end
    on_intel do
$mac_intel
    end
  end

  on_linux do
    on_arm do
$linux_arm
    end
    on_intel do
$linux_intel
    end
  end

  def install
    bin.install "mustty"
  end

  test do
    assert_match "mustty #{version}", shell_output("#{bin}/mustty --version")
  end
end
RUBY
