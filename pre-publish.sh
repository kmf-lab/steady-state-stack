#!/bin/bash

# Removed niceness adjustment for faster execution.
# If system responsiveness is a concern, you can re-enable it:
# if [ "$NICENESS_SET" != "1" ]; then
#     export NICENESS_SET=1
#     sudo exec nice -n 10 "$0" "$@"
# fi

# Update dependencies and set Rust toolchain to stable
# Note: Consider running cargo update less frequently if dependencies don't change often
cargo update
rustup default stable

# Check for unwanted crates in a single cargo tree call to save time
unwanted_crates="tokio smol actix rocket warp"
cargo_tree_output=$(cargo tree)
for crate in $unwanted_crates; do
    if echo "$cargo_tree_output" | grep -q "$crate"; then
        echo "Error: '$crate' crate found in the Cargo project."
        exit 1
    else
        echo "Success: No '$crate' crate found in the Cargo project."
    fi
done

# Run tests with optimized threads
# Adjust RUST_TEST_THREADS based on your system's core count for optimal performance (e.g., number of physical cores)
RUST_BACKTRACE=full RUST_LOG=debug RUST_TEST_THREADS=4 cargo test --workspace --tests -- --nocapture --show-output | tee cargo_test.txt
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Tests failed with exit code $exit_code"
    exit $exit_code
fi

# Build the workspace in offline mode, skipping tests and examples if not needed
# If tests and examples are required for release, add --tests --examples back
RUST_BACKTRACE=1 cargo build --offline --workspace | tee cargo_build.txt
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Build failed with exit code $exit_code"
    exit $exit_code
fi

# Build release version with specific features and parallel jobs
# Adjust -j flag to match your CPU's core count for faster compilation (e.g., -j 8 for 8 cores)
RUST_BACKTRACE=1 cargo build --offline --release --workspace --features "proactor_nuclei telemetry_server_cdn" -j 4 | tee cargo_build_release.txt
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Release build failed with exit code $exit_code"
    exit $exit_code
fi

# Build documentation like on docs.rs server
# Consider running this only when documentation changes to save time
cd core
RUSTDOCFLAGS="--cfg=docsrs" cargo rustdoc --features "proactor_nuclei" --no-default-features | tee cargo_rustdoc.txt
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Docs build failed with exit code $exit_code"
    exit $exit_code
fi
cd ..

# Optional: List directory structure excluding target
tree -I 'target'

# Run cargo outdated and audit
# Consider running these periodically rather than before every release to save time
echo "cargo outdated"
cargo outdated | tee cargo_outdated.txt

echo "cargo audit"
cargo audit | tee cargo_audit.txt

# Install current cargo-steady-state
# Ensure this is necessary for your release process
echo "install current cargo-steady-state"
cargo install --path cargo-steady-state

# Optional: Run coverage and statistics
# These can be skipped or run separately to save time
cargo llvm-cov -- --nocapture --show-output
echo "To generate coverage report: cargo llvm-cov --html --output-dir coverage/"

echo "cargo tree"
tokei | tee cargo_tokei.txt

# Final confirmation message
echo "Confirm that warnings you do not want published have been removed"
echo "If this is confirmed by successful GitHub build !!, you may now run: cargo publish"
