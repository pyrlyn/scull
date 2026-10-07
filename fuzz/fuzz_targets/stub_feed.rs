//! libFuzzer target for the conformance stub. Arbitrary bytes must leave the
//! grid size alone. Separate from the crate tests so a nightly fuzz run can
//! hammer the same invariant the fixed corpus checks in CI.

#![no_main]

use libfuzzer_sys::fuzz_target;

use scull_harness::Stub;

fuzz_target!(|data: &[u8]| {
    // Size stays fixed: a fuzzer-chosen width could ask for the whole
    // address space, and the property under test is that `feed` does not resize.
    let mut stub = Stub::new(80, 24);
    let cols = stub.cols();
    let rows = stub.rows();
    stub.feed(data);
    assert_eq!(stub.cols(), cols);
    assert_eq!(stub.rows(), rows);
});
