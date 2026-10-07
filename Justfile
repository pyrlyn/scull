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

# Baseline table for the stub and the reference terminals.
bench:
    mise exec -- cargo run -p scull-bench --release --locked

# Rewrite the committed C header and C# bindings from the exports.
bindings:
    SCULL_BLESS=1 mise exec -- cargo nextest run -p scull-ffi --test bindings --locked

# Fail when the committed C header or C# bindings are stale (also part of `test`).
bindings-check:
    mise exec -- cargo nextest run -p scull-ffi --test bindings --locked

c_abi_out := "target/c-abi"
c_abi_cc := "cc -std=c11 -g -O1 -fno-omit-frame-pointer -Wall -Wextra -Werror -Icrates/scull-ffi/include"
c_abi_link := "-Ltarget/debug -lscull_ffi -Wl,-rpath," + justfile_directory() + "/target/debug -lpthread"

# Build the C ABI as a shared library and drive it from C on two threads,
# under AddressSanitizer with UndefinedBehaviorSanitizer, then under
# ThreadSanitizer. Unix only: the sanitizers instrument the C side, which
# is where a host's misuse of the ABI would show. A just recipe, not a
# test, because building C means running a compiler, and only scull-pty
# may spawn processes (clippy.toml).
c-abi-test:
    mise exec -- cargo rustc -p scull-ffi --lib --crate-type cdylib --locked
    mkdir -p {{c_abi_out}}
    {{c_abi_cc}} -fsanitize=address,undefined -fno-sanitize-recover=all crates/scull-ffi/tests/c/two_threads.c {{c_abi_link}} -o {{c_abi_out}}/two_threads-asan
    ./{{c_abi_out}}/two_threads-asan
    {{c_abi_cc}} -fsanitize=thread crates/scull-ffi/tests/c/two_threads.c {{c_abi_link}} -o {{c_abi_out}}/two_threads-tsan
    ./{{c_abi_out}}/two_threads-tsan

macos_app := "target/macos/Scull.app"

# The core as a static library for the Swift package to link; arm64 only.
# `cargo rustc` picks the crate type here so the manifest stays as is.
[private]
macos-lib features="":
    mise exec -- cargo rustc -p scull-ffi --lib --crate-type staticlib --release --target aarch64-apple-darwin --locked {{ if features == "" { "" } else { "--features " + features } }}

# Build the macOS app into target/macos/Scull.app, signed ad hoc so it runs
# locally. The Swift side is a debug build, which keeps the scripted-launch
# hook (`open target/macos/Scull.app --args -ScullInitialInput ls`).
macos: macos-lib
    cd macos && swift build --arch arm64
    rm -rf {{macos_app}}
    mkdir -p {{macos_app}}/Contents/MacOS {{macos_app}}/Contents/Resources
    cp macos/Info.plist {{macos_app}}/Contents/
    cp macos/Resources/* {{macos_app}}/Contents/Resources/
    cp "$(cd macos && swift build --arch arm64 --show-bin-path)/Scull" {{macos_app}}/Contents/MacOS/
    codesign --force --sign - {{macos_app}}

# The Swift package's tests. The library carries the `test-hooks` feature so
# a test can poison one terminal; `just macos` rebuilds it without.
macos-test:
    just macos-lib test-hooks
    cd macos && swift test --arch arm64
