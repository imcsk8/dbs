test:
	cargo test

run:
	cargo run

debug:
	LOG_LEVEL=debug cargo run

clean:
	cargo clean

release:
	cargo build --release; \
		objcopy --compress-debug-sections target/release/pipeline ./bin/main

