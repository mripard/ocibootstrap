#[test]
fn num_cast() {
    assert_eq!(ocibootstrap_part::num_cast!(u64, 42_usize), 42_u64);
}

#[test]
#[should_panic]
fn num_cast_overflow() {
    ocibootstrap_part::num_cast!(u32, u64::from(u32::MAX) + 1);
}

#[test]
#[should_panic]
fn num_cast_underflow() {
    ocibootstrap_part::num_cast!(u32, -1_i32);
}
