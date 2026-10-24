#!/bin/sh
# Installs the latest Yapkeeper release for this machine:
#
#   curl -fsSL https://raw.githubusercontent.com/julianbonomini/yapkeeper/main/install.sh | sh
#
# YAPKEEPER_VERSION=v0.1.1 installs that release instead, and
# YAPKEEPER_INSTALL_DIR changes where the binary goes (default ~/.local/bin).
set -eu

repo=julianbonomini/yapkeeper
dir=${YAPKEEPER_INSTALL_DIR:-$HOME/.local/bin}

case "$(uname -s)" in
	Darwin) os=apple-darwin ;;
	Linux) os=unknown-linux-musl ;;
	*)
		echo "yapkeeper: there is no release for $(uname -s); build it with cargo instead." >&2
		exit 1
		;;
esac
case "$(uname -m)" in
	arm64 | aarch64) arch=aarch64 ;;
	x86_64 | amd64) arch=x86_64 ;;
	*)
		echo "yapkeeper: there is no release for $(uname -m); build it with cargo instead." >&2
		exit 1
		;;
esac

if [ -n "${YAPKEEPER_VERSION:-}" ]; then
	base="https://github.com/$repo/releases/download/$YAPKEEPER_VERSION"
else
	base="https://github.com/$repo/releases/latest/download"
fi
name="yapkeeper-$arch-$os"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $name.tar.gz"
curl -fsSL -o "$tmp/$name.tar.gz" "$base/$name.tar.gz"
curl -fsSL -o "$tmp/$name.tar.gz.sha256" "$base/$name.tar.gz.sha256"
(
	cd "$tmp"
	if command -v sha256sum > /dev/null; then
		sha256sum -c --quiet "$name.tar.gz.sha256"
	else
		shasum -a 256 -c --quiet "$name.tar.gz.sha256"
	fi
)
tar -xzf "$tmp/$name.tar.gz" -C "$tmp"

mkdir -p "$dir"
cp "$tmp/$name/yapkeeper" "$dir/yapkeeper"
chmod 755 "$dir/yapkeeper"
echo "Installed $("$dir/yapkeeper" --version) to $dir/yapkeeper"

case ":$PATH:" in
	*":$dir:"*) ;;
	*) echo "$dir is not on your PATH; add it to run yapkeeper by name." ;;
esac
