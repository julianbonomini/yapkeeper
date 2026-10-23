# Checks the Rust workspace before review and merge.
# Compiler lint flags apply to workspace targets rather than dependency internals.
set -eu

cd "$(dirname "$0")/../.."

if [ ! -f Cargo.toml ]; then
	echo "Cargo.toml is missing. Initialize the Rust project before running checks." >&2
	exit 1
fi

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings -F unsafe-code
cargo test --workspace --all-features
