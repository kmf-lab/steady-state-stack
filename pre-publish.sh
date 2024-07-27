#!/bin/bash

cargo update

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


cargo build --workspace --examples
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Tests failed with exit code $exit_code"
    exit $exit_code
fi

# micro release without builtin viz
cargo build --release --workspace --examples --features "proactor_nuclei telemetry_server_cdn"
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Tests failed with exit code $exit_code"
    exit $exit_code
fi

RUST_TEST_THREADS=6 cargo test
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Tests failed with exit code $exit_code"
    exit $exit_code
fi

RUST_TEST_THREADS=6 cargo test --workspace --examples
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Tests failed with exit code $exit_code"
    exit $exit_code
fi

# Build documentation like on docs.rs server
cd core
RUSTDOCFLAGS="--cfg=docsrs" cargo rustdoc --features "proactor_nuclei" --no-default-features
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Docs build failed with exit code $exit_code"
    exit $exit_code
fi
cd ..

# Run tarpaulin for code coverage
# cargo install cargo-tarpaulin --force
echo "RUST_TEST_THREADS=3 cargo tarpaulin --timeout 180 --out html --no-fail-fast --tests --examples"
RUST_TEST_THREADS=3 cargo tarpaulin --timeout 180 --out Stdout --no-fail-fast --tests --examples || true
 #  --output-dir target/tarpaulin-report  --no-fail-fast


tree -I 'target'


echo "cargo outdated"
cargo outdated | tee cargo_outdated.txt


echo "cargo audit"
cargo audit | tee cargo_audit.txt


echo "cargo tree"
tokei | tee cargo_tokei.txt

echo "If this is confirmed by GitHub build YOU may now run:   cargo publish"



