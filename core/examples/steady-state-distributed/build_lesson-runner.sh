#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

# Define the root directory for the search
ROOT_DIR="runner"
# Define the output file in the original directory
output_file="lesson-02C-distributed-runner.md"
# Define the README path in the original directory
README_PATH="README.md"

# Function to generate directory tree, excluding 'target' and hidden directories
get_directory_tree() {
    local path=$1
    local prefix=$2
    local stack=("$path")
    local visited=()

    while [[ ${#stack[@]} -gt 0 ]]; do
        local current=${stack[-1]}
        local dirs=()
        unset "stack[-1]"

        # Use find with -print0 and process with xargs to handle null bytes correctly
        local files=($(find "$current" -mindepth 1 -maxdepth 1 \
            -not -name 'target' -not -name '.*' -not -type l -print0 | xargs -0))

        for ((i=0; i<${#files[@]}; i++)); do
            local file=${files[$i]}
            local base=$(basename "$file")
            local is_dir=$(test -d "$file" && echo true || echo false)
            local is_last=$((${#files[@]} - 1 == $i))

            if [[ $is_last == true ]]; then
                item_prefix='\-- '
                new_prefix="$prefix    "
            else
                item_prefix='+-- '
                new_prefix="$prefix|   "
            fi

            echo "$prefix$item_prefix$base"

            if [[ $is_dir == true ]]; then
                stack+=("$file")
            fi
        done
    done
}

# Initialize output file, including README.md if it exists
if [ -f "$README_PATH" ]; then
    cat "$README_PATH" > "$output_file"
    echo "" >> "$output_file"
    echo "---" >> "$output_file"
    echo "" >> "$output_file"
else
    : > "$output_file"
fi

# Change to the specified root directory
pushd "$ROOT_DIR" > /dev/null

# Add project structure section
{
    echo '# Lesson 02C: Distributed Runner'
    echo ''
    echo '## Project Structure'
    echo ''
    echo '```'
    echo '.'
    get_directory_tree '.' ''
    echo '```'
    echo ''
} >> "$output_file"

# Process each Cargo.toml and associated .rs and .toml files
cargo_tomls=$(find . -type f -name Cargo.toml | sort)
for toml in $cargo_tomls; do
    project_dir=$(dirname "$toml")
    files=$(find "$project_dir" -type f \( -name '*.rs' -o -name '*.toml' \) \
        -not -path '*/target/*' -not -path '*/.*/*' | sort)
    for file in $files; do
        relative_path=$(realpath --relative-to=.. "$file")
        ext="${file##*.}"
        # Add file header
        echo "## $relative_path" >> "$output_file"
        echo '' >> "$output_file"
        # Directly echo the code block delimiter based on extension
        case "$ext" in
            rs) echo '```rust' ;;
            toml) echo '```toml' ;;
            *) echo '```text' ;;
        esac >> "$output_file"
        # Append file content and ensure it ends with a newline
        cat "$file" >> "$output_file"
        if [ -s "$file" ] && [ "$(tail -c 1 "$file")" != $'\n' ]; then
            echo >> "$output_file"
        fi
        # Close code block and add spacing
        echo '```' >> "$output_file"
        echo '' >> "$output_file"
    done
done

# Restore the original directory
popd > /dev/null

# Create a .txt copy if the output file is not already a .txt file
if [[ "$output_file" != *.txt ]]; then
    cp "$output_file" "../lesson-02C-distributed-runner.md.txt"
fi