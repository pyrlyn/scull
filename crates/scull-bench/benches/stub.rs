//! Divan samples of `scull_harness::Stub::feed` on the fixed benchmark input.
//! The committed baseline is the wall-clock table from the binary.

use divan::Bencher;

fn main() {
    divan::main();
}

#[divan::bench]
fn stub_feed(bencher: Bencher) {
    let input = scull_bench::fixed_input();
    bencher.bench_local(|| scull_bench::feed_stub(divan::black_box(&input)));
}
