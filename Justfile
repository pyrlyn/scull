# The gate every change passes before merge (rust.md: "Before you call it done").
check: fmt-check lint test

fmt-check:
    mise exec -- cargo fmt --all --check

lint:
    mise exec -- cargo clippy --workspace --all-targets --locked -- -D warnings

test:
    mise exec -- cargo nextest run --workspace --all-targets --locked --no-tests=pass

# Regenerate scull-unicode's tables from the pinned Unicode data files,
# downloading them once into the gitignored target/ucd/ cache.
ucd:
    mise exec -- cargo run --locked -p scull-ucd-gen

# Fail when the committed tables differ from what the pinned data files produce.
ucd-check:
    mise exec -- cargo run --locked -p scull-ucd-gen -- --check
