// Depending on the crate here forces it to link.
use concurrent_roaring as _;

#[test]
fn links_crate() {
    assert_eq!(1 + 1, 2);
}
