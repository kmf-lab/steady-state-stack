#!/bin/bash

# Check if the output contains 'tokio'
if echo "$(cargo tree -i tokio 2>&1)" | grep -q "did not match any packages"; then
    echo "Success: No 'tokio' crate found in the Cargo project."
else
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

# new pre-publish script to be run
cargo test
cargo test --workspace --examples
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Tests failed with exit code $exit_code"
    exit $exit_code
fi


# cargo bloat --release | tee cargo_bloat.txt
cargo outdated | tee cargo_outdated.txt
cargo audit | tee cargo_audit.txt
tokei | tee cargo_tokei.txt

echo "If this is confirmed by GitHub build YOU may now run:   cargo publish";


