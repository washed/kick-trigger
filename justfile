build-log:
	cargo build -p kick-trigger --release -F logging -F detector-diagnostics --target thumbv7em-none-eabihf --locked

run-log:
	cargo run -p kick-trigger --release -F logging -F detector-diagnostics --target thumbv7em-none-eabihf --locked

build:
	cargo build -p kick-trigger --release --target thumbv7em-none-eabihf --locked

run:
	cargo run -p kick-trigger --release --target thumbv7em-none-eabihf --locked
