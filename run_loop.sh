#!/bin/bash
cargo clean
# Background build loop
while true; do
  cargo build --workspace --all-targets > /dev/null 2>&1
  cargo clean > /dev/null 2>&1
done &
BUILD_PID=$!

for i in $(seq 1 200); do
  echo "Run $i"
  # Run the test and capture output
  USERNAME=htui-ci cargo test -p htui --features testkit --test auth > test_out.log 2>&1
  if ! grep -q "test result: ok. 5 passed" test_out.log; then
    echo "FAILED ON RUN $i"
    cat test_out.log
    kill $BUILD_PID
    exit 1
  fi
done
kill $BUILD_PID
echo "Completed 200 runs without failure"
