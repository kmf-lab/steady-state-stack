#!/bin/bash

# You may need to select Java 17
# sudo update-alternatives --config java


# Variables
AERON_REPO="https://github.com/real-logic/aeron.git"
AERON_BRANCH="1.39.0"
OUT_DIR="$(pwd)/target/aeron-driver-native"
BUILD_DIR="$OUT_DIR/build"

# Check for required tools
function check_required_tools {
    for tool in git gcc cmake make; do
        if ! command -v $tool &>/dev/null; then
            echo "Error: $tool is not installed."
            exit 1
        fi
    done
}

# Clone Aeron C++ Media Driver repository
function clone_aeron {
    if [ ! -d "$OUT_DIR" ]; then
        echo "Cloning Aeron C++ Media Driver..."
        git clone --depth 1 --branch $AERON_BRANCH $AERON_REPO "$OUT_DIR" || {
            echo "Failed to clone Aeron repository."
            exit 1
        }
    else
        echo "Aeron repository already cloned."
    fi
}

# Build Aeron C++ Media Driver
function build_aeron {

    # check out appears to be for JDK 11
    # so we bump up to JDK 17, not yet compatible with 21
    mkdir -p "$OUT_DIR"
      cd "$OUT_DIR" || exit 1
    ./gradlew wrapper --gradle-version 7.4.2


    echo "Building Aeron C++ Media Driver..."

    mkdir -p "$BUILD_DIR"
    cd "$BUILD_DIR" || exit 1

    # Run CMake configuration
    cmake .. -DBUILD_SHARED_LIBS=ON || {
        echo "CMake configuration failed."
        exit 1
    }

    # Build the project
    cmake --build . || {
        echo "Build failed."
        exit 1
    }

    echo "Aeron C++ Media Driver built successfully!"
}

# Main script
check_required_tools
clone_aeron
build_aeron
find . -name aeronmd

