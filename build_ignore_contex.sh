#!/bin/bash
TARGET_FOLDER=../review_content/steady_state_stack
bash ../source_context_builder.sh project_root ./,./htmlReport,./review $TARGET_FOLDER
bash ../source_context_builder.sh core_shared ./core,./core/templates $TARGET_FOLDER
bash ../source_context_builder.sh cargo_tool ./cargo-steady-state,./cargo-steady-state/src,./cargo-steady-state/templates $TARGET_FOLDER
bash ../source_context_builder.sh distributed ./core/src/distributed $TARGET_FOLDER
bash ../source_context_builder.sh telemetry ./core/src/telemetry,./core/static/telemetry,./core/static/telemetry/images,./core/static/telemetry/images/old $TARGET_FOLDER
bash ../source_context_builder.sh serialize_and_install ./core/src/serialize,./core/src/install $TARGET_FOLDER
bash ../source_context_builder.sh routing_services ./core/routing_service/mqtt,./core/routing_service/aeron $TARGET_FOLDER
bash ../source_context_builder.sh widget_examples ./core/examples/simple_widgets,./core/examples/simple_widgets/actor,./core/examples/large_scale,./core/examples/large_scale/actor $TARGET_FOLDER
bash ../source_context_builder.sh performance_examples ./core/examples/high_performance,./core/examples/high_performance/actor,./core/examples/high_durability,./core/examples/high_durability/actor $TARGET_FOLDER
bash ../source_context_builder.sh aeron_and_fizz_buzz_examples ./core/examples/simple_aeron_publisher,./core/examples/simple_aeron_publisher/actor,./core/examples/simple_aeron_subscriber,./core/examples/simple_aeron_subscriber/actor,./core/examples/fizz_buzz,./core/examples/fizz_buzz/actor $TARGET_FOLDER
