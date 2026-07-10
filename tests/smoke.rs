// Depending on the crate here forces it to link; the test passing is the P0 harness check.
use concurrent_roaring as _;

#[test]
fn links_crate() {
    assert_eq!(1 + 1, 2);
}
