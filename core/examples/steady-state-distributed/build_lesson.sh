#!/bin/bash

echo "Running build_lesson-publish.sh"
bash build_lesson-publish.sh

echo "Running build_lesson-runner.sh"
bash build_lesson-runner.sh

echo "Running build_lesson-subscribe.sh"
bash build_lesson-subscribe.sh
