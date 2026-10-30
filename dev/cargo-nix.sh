#!/usr/bin/env bash
# Regenerate Cargo.nix (one Nix derivation per crate, nix/package.nix) and
# the stamp the build compares against the manifests. Run after any change
# to a Cargo.toml or Cargo.lock; the pre-commit hook runs it for you.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
nix shell nixpkgs#crate2nix nixpkgs#cargo --command crate2nix generate >/dev/null
# The same bytes, in the same order, as nix/package.nix hashes: the root
# manifest, the lock, then each crate's manifest by directory name.
{
  cat Cargo.toml Cargo.lock
  for c in $(LC_ALL=C ls crates); do cat "crates/$c/Cargo.toml"; done
} | sha256sum | cut -d' ' -f1 > nix/cargo-nix.stamp
