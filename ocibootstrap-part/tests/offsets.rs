#![allow(missing_docs)]

use log as _;
use num_traits as _;
use test_log::test;

#[test]
fn start_end_to_size() {
    assert_eq!(ocibootstrap_part::start_end_to_size(10, 19), 10);
}

#[test]
#[should_panic]
fn start_end_to_size_end_before_start() {
    let _ = ocibootstrap_part::start_end_to_size(10, 9);
}

#[test]
fn start_end_to_size_start_equal_end() {
    assert_eq!(ocibootstrap_part::start_end_to_size(10, 10), 1);
}

#[test]
fn start_end_to_size_zero_zero() {
    assert_eq!(ocibootstrap_part::start_end_to_size(0, 0), 1);
}

#[test]
#[should_panic]
fn start_end_to_size_negative_start() {
    let _ = ocibootstrap_part::start_end_to_size(-1, 9);
}

#[test]
#[should_panic]
fn start_end_to_size_negative_end() {
    let _ = ocibootstrap_part::start_end_to_size(10, -2);
}

#[test]
fn start_size_to_end() {
    assert_eq!(ocibootstrap_part::start_size_to_end(10, 10), 19);
}

#[test]
fn start_size_to_end_one_size() {
    assert_eq!(ocibootstrap_part::start_size_to_end(10, 1), 10);
}

#[test]
#[should_panic]
fn start_size_to_end_negative_start() {
    let _ = ocibootstrap_part::start_size_to_end(-10, 9);
}

#[test]
#[should_panic]
fn start_size_to_end_negative_size() {
    let _ = ocibootstrap_part::start_size_to_end(10, -9);
}

#[test]
#[should_panic]
fn start_size_to_end_null_size() {
    let _ = ocibootstrap_part::start_size_to_end(10, 0);
}

#[test]
fn size_end_to_start() {
    assert_eq!(ocibootstrap_part::size_end_to_start(10, 19), 10);
}

#[test]
fn size_end_to_start_zero_start() {
    assert_eq!(ocibootstrap_part::size_end_to_start(10, 9), 0);
}

#[test]
#[should_panic]
fn size_end_to_start_larger_end() {
    let _ = ocibootstrap_part::size_end_to_start(20, 10);
}

#[test]
#[should_panic]
fn size_end_to_start_negative_size() {
    let _ = ocibootstrap_part::size_end_to_start(-10, 9);
}

#[test]
#[should_panic]
fn size_end_to_start_negative_end() {
    let _ = ocibootstrap_part::size_end_to_start(10, -9);
}

#[test]
#[should_panic]
fn size_end_to_start_null_size() {
    let _ = ocibootstrap_part::size_end_to_start(0, 9);
}
