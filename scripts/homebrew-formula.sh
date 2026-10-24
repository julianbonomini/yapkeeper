#!/bin/sh
# Prints the Homebrew formula for a release.
# Usage: homebrew-formula.sh <version> <folder holding the release's .sha256 files>
set -eu

version=$1
dir=$2
url="https://github.com/julianbonomini/yapkeeper/releases/download/v$version"

# The url and sha256 lines for one release target.
asset() {
	file="yapkeeper-$1.tar.gz"
	sum=$(cut -d ' ' -f 1 "$dir/$file.sha256")
	printf '      url "%s/%s"\n      sha256 "%s"\n' "$url" "$file" "$sum"
}

cat <<FORMULA
class Yapkeeper < Formula
  desc "Find, browse and resume Claude Code, Codex and Pi sessions"
  homepage "https://github.com/julianbonomini/yapkeeper"
  version "$version"
  license "MIT"

  on_macos do
    on_arm do
$(asset aarch64-apple-darwin)
    end
    on_intel do
$(asset x86_64-apple-darwin)
    end
  end

  on_linux do
    on_arm do
$(asset aarch64-unknown-linux-musl)
    end
    on_intel do
$(asset x86_64-unknown-linux-musl)
    end
  end

  def install
    bin.install "yapkeeper"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/yapkeeper --version")
  end
end
FORMULA
