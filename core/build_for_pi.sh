#!/bin/bash

export CC_arm_linux_gnueabihf=arm-linux-gnueabihf-gcc
export CXX_arm_linux_gnueabihf=arm-linux-gnueabihf-g++

cargo clean

sudo apt-get update

sudo apt-get install gcc-arm-linux-gnueabihf

rustup target add armv7-unknown-linux-gnueabihf

sudo apt-get install crossbuild-essential-armhf

cargo build --target=armv7-unknown-linux-gnueabihf
