# JanusOS build entry points.
#
#   make run             demo dashboard (janus-web, port 5000)
#   make runtime         production engine (janus-runtime, loopback:8080)
#   make check           type-check the whole Rust workspace
#   make test            Rust tests
#   make android         debug APK for the CM5 (arm64)
#   make android-release release APK with the full module catalogue
#
# The Android targets need cargo-ndk, the NDK and a JDK; see docs/ANDROID.md.
# The ISO/server targets are unchanged (iso/, deploy/).

run:
	cargo run --bin janus-web

runtime:
	cargo run --bin janus-runtime -- --config config/janus.toml

release:
	cargo build --release --workspace

check:
	cargo check --workspace

test:
	cargo test --workspace

clean:
	cargo clean

watch:
	cargo watch -x 'run --bin janus-web'

inventory:
	./scripts/validate-modules.sh

android:
	./scripts/build-android.sh

android-release:
	./scripts/build-android.sh --release --full --stage

.PHONY: run runtime release check test clean watch inventory android android-release
