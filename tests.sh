cargo fmt --all --check
cargo test --quiet --locked
cargo clippy -- --deny warnings
cargo clippy --tests -- --deny warnings
find . -name '*.nix' -exec nix fmt {} \+
nix flake check
