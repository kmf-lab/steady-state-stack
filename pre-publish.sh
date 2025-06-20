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

if sccache --version; then
  echo "USING sccache ---------------------------------------"
  export RUSTC_WRAPPER=sccache
fi


# Check for unwanted crates in a single cargo tree call to save time
unwanted_crates="tokio actix rocket warp"
cargo_tree_output=$(cargo tree)
for crate in $unwanted_crates; do
    if echo "$cargo_tree_output" | grep -q "$crate"; then
        echo "Error: '$crate' crate found in the Cargo project."
        cargo tree | -B 15 '$crate'
        exit 1
    else
        echo "Success: No '$crate' crate found in the Cargo project."
    fi
done

# Run tests with optimized threads
# Adjust RUST_TEST_THREADS based on your system's core count for optimal performance (e.g., number of physical cores)
#RUST_BACKTRACE=full RUST_LOG=debug RUST_TEST_THREADS=4 cargo test --workspace --tests -- --nocapture --show-output | tee cargo_test.txt
#exit_code=$?
#if [ $exit_code -ne 0 ]; then
#    echo "Tests failed with exit code $exit_code"
#    exit $exit_code
#fi
# Ensure cargo-nextest is installed
if ! command -v cargo-nextest &> /dev/null; then
    echo "cargo-nextest not found, installing..."
    cargo install cargo-nextest
fi

# Run tests with cargo-nextest, optimizing threads automatically
# RUST_TEST_THREADS is not needed as nextest manages parallelism itself
# Use --test-threads to manually override if desired (e.g., --test-threads=4)
RUST_BACKTRACE=full RUST_LOG=debug cargo nextest run --workspace --examples --tests | tee cargo_test.txt
exit_code=$?

if [ $exit_code -ne 0 ]; then
    echo "Tests failed with exit code $exit_code"
    exit $exit_code
fi
echo "====================================================================================================================="
echo "============ success all tests ran well ============================================================================="
echo "====================================================================================================================="


# Build the workspace in offline mode, skipping tests and examples if not needed
# If tests and examples are required for release, add --tests --examples back
RUST_BACKTRACE=1 cargo build --offline --workspace | tee cargo_build.txt
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Build failed with exit code $exit_code"
    exit $exit_code
fi

# Build release version
# Adjust -j flag to match your CPU's core count for faster compilation (e.g., -j 8 for 8 cores)
RUST_BACKTRACE=1 cargo build --release --workspace --examples --tests -j 12 | tee cargo_build_release.txt
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Release build failed with exit code $exit_code"
    exit $exit_code
fi

# Build documentation like on docs.rs server
# Consider running this only when documentation changes to save time
cd core
RUSTDOCFLAGS="--cfg=docsrs" cargo rustdoc | tee cargo_rustdoc.txt
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
cargo install cargo-outdated
echo "cargo outdated"
cargo outdated | tee cargo_outdated.txt

echo "cargo audit"
cargo audit | tee cargo_audit.txt

# Install current cargo-steady-state
# Ensure this is necessary for your release process
echo "install current cargo-steady-state"
cargo install --path cargo-steady-state

echo "---------------------------------------------------------------------------------"
echo "------------------------    compute coverage please wait ------------------------"
echo "---------------------------------------------------------------------------------"
# Optional: Run coverage and statistics
# These can be skipped or run separately to save time
cargo llvm-cov --lcov --output-path cov_a.lcov --no-default-features -F exec_async_std,telemetry_server_builtin,core_affinity,core_display,prometheus_metrics
cargo llvm-cov --lcov --output-path cov_b.lcov --no-default-features -F proactor_nuclei,telemetry_server_cdn
lcov --add-tracefile cov_a.lcov --add-tracefile cov_b.lcov -o merged.lcov
genhtml merged.lcov --output-directory coverage_html



# cargo llvm-cov nextest --no-default-features -F exec_async_std,telemetry_server_builtin,core_affinity,core_display,prometheus_metrics
#echo "To generate coverage report: cargo llvm-cov --html --output-dir coverage/"

echo "cargo tree"
tokei | tee cargo_tokei.txt

# Final confirmation message
echo "Confirm that warnings you do not want published have been removed"
echo "If this is confirmed by successful GitHub build !!, you may now run: cargo publish"

# cargo install sccache
# export RUSTC_WRAPPER=sccache
#[build]
#rustc-wrapper = "sccache"

# sudo apt install mold
# export RUSTFLAGS="-C link-arg=-fuse-ld=mold"
#[target.x86_64-unknown-linux-gnu]
#linker = "mold"

# cargo install cargo-nextest
# cargo nextest run
