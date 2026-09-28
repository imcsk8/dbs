dirs:
	mkdir -p bin

build:
	cargo build

test:
	cargo test -- --test-threads=1

lint:
	cargo clippy

run:
	cargo run

debug:
	LOG_LEVEL=debug cargo run

clean:
	cargo clean

release: dirs
	cargo build --release
	mkdir -p bin
	install -m 755 target/release/dbs bin/dbs
