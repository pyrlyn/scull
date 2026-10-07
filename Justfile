# The gate every change passes before merge (rust.md: "Before you call it done").
check: fmt-check lint test

fmt-check:
    mise exec -- cargo fmt --all --check

lint:
    mise exec -- cargo clippy --workspace --all-targets --locked -- -D warnings

test:
    mise exec -- cargo nextest run --workspace --all-targets --locked --no-tests=pass
