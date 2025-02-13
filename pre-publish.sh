#!/bin/bash

if [ "$NICENESS_SET" != "1" ]; then
    export NICENESS_SET=1
    sudo exec nice -n 10 "$0" "$@"
fi

cargo update
rustup default stable


# Check if the output contains 'tokio'
if echo "$(cargo tree -i tokio 2>&1)" | grep -q "did not match any packages"; then
    echo "Success: No 'tokio' crate found in the Cargo project."
else
    cargo tree -i tokio
    echo "Error: 'tokio' crate found in the Cargo project."
    exit 1
fi

# Check if the output contains 'smol'
if echo "$(cargo tree -i smol 2>&1)" | grep -q "did not match any packages"; then
    echo "Success: No 'smol' crate found in the Cargo project."
else
    echo "Error: 'smol' crate found in the Cargo project."
    exit 1
fi

# Check if the output contains 'actix'
if echo "$(cargo tree -i actix 2>&1)" | grep -q "did not match any packages"; then
    echo "Success: No 'actix' crate found in the Cargo project."
else
    echo "Error: 'actix' crate found in the Cargo project."
    exit 1
fi

# Check if the output contains 'rocket'
if echo "$(cargo tree -i rocket 2>&1)" | grep -q "did not match any packages"; then
    echo "Success: No 'rocket' crate found in the Cargo project."
else
    echo "Error: 'rocket' crate found in the Cargo project."
    exit 1
fi

# Check if the output contains 'warp'
if echo "$(cargo tree -i warp 2>&1)" | grep -q "did not match any packages"; then
    echo "Success: No 'warp' crate found in the Cargo project."
else
    echo "Error: 'warp' crate found in the Cargo project."
    exit 1
fi

# still considering if I want to ban protobuf usage
# Check if the output contains 'protocol buffers'
# cargo tree -i protobuf
# cargo tree -i prost
# cargo tree -i prost-build

RUST_BACKTRACE=full RUST_LOG=debug RUST_TEST_THREADS=24 cargo test --workspace --tests --examples -j 24 -- --nocapture --show-output | tee cargo_test.txt
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Tests failed with exit code $exit_code"
    exit $exit_code
fi

RUST_BACKTRACE=1 cargo build --workspace --tests --examples -j 48 | tee cargo_build.txt
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Tests failed with exit code $exit_code"
    exit $exit_code
fi

# micro release without builtin viz
RUST_BACKTRACE=1 cargo build --release --workspace --examples --features "proactor_nuclei telemetry_server_cdn" -j 12 | tee cargo_build_release.txt
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Tests failed with exit code $exit_code"
    exit $exit_code
fi




# Build documentation like on docs.rs server
cd core
RUSTDOCFLAGS="--cfg=docsrs" cargo rustdoc --features "proactor_nuclei" --no-default-features | tee cargo_rustdoc.txt
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Docs build failed with exit code $exit_code"
    exit $exit_code
fi
cd ..

tree -I 'target'

echo "cargo outdated"
cargo outdated | tee cargo_outdated.txt

echo "cargo audit"
cargo audit | tee cargo_audit.txt

echo "install current cargo-steady-state"
cargo install --path cargo-steady-state

cargo llvm-cov
echo "cargo llvm-cov --html --output-dir coverage/"

echo "cargo tree"
tokei | tee cargo_tokei.txt

echo "Confirm that warnings you do not want published have been removed"
echo "If this is confirmed by successful GitHub build YOU may now cd and run:   cargo publish"


