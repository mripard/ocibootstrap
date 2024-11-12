#[test]
fn div_round_up() {
    assert_eq!(ocibootstrap_part::div_round_up(42, 10), 5);
}

#[test]
fn div_round_up_aligned() {
    assert_eq!(ocibootstrap_part::div_round_up(42, 14), 3);
}

#[test]
fn div_round_up_one_aligned() {
    assert_eq!(ocibootstrap_part::div_round_up(42, 1), 42);
}

#[test]
#[should_panic]
fn div_round_up_null_multiple() {
    let _ = ocibootstrap_part::div_round_up(42, 0);
}

#[test]
fn round_up() {
    assert_eq!(ocibootstrap_part::round_up(42, 10), 50);
}

#[test]
fn round_up_already_aligned() {
    assert_eq!(ocibootstrap_part::round_up(42, 14), 42);
}

#[test]
fn round_up_one_aligned() {
    assert_eq!(ocibootstrap_part::round_up(42, 1), 42);
}

#[test]
#[should_panic]
fn round_up_null_multiple() {
    let _ = ocibootstrap_part::round_up(42, 0);
}

#[test]
fn round_down() {
    assert_eq!(ocibootstrap_part::round_down(42, 10), 40);
}

#[test]
fn round_down_already_aligned() {
    assert_eq!(ocibootstrap_part::round_down(42, 14), 42);
}

#[test]
fn round_down_one_aligned() {
    assert_eq!(ocibootstrap_part::round_down(42, 1), 42);
}

#[test]
#[should_panic]
fn round_down_null_multiple() {
    let _ = ocibootstrap_part::round_down(42, 0);
}
