//! The vendored rarest-first chooser; the vendored crate's own tests don't run.

use librqbit::riptor_pick::pick_rarest;

fn pick(
    groups: Vec<Vec<usize>>,
    has: impl Fn(usize) -> bool,
    avail: &[u16],
    salt: u64,
) -> Option<usize> {
    pick_rarest(groups, |p| p, has, avail, salt)
}

#[test]
fn takes_the_rarest_piece_the_peer_has() {
    let avail = [5, 1, 3, 2, 9];
    let all = || vec![vec![0, 1, 2, 3, 4]];
    assert_eq!(pick(all(), |_| true, &avail, 0), Some(1));
    assert_eq!(pick(all(), |p| p != 1, &avail, 0), Some(3));
    assert_eq!(pick(all(), |p| p == 0 || p == 4, &avail, 0), Some(0));
    assert_eq!(pick(all(), |_| false, &avail, 0), None);
    assert_eq!(pick(vec![], |_| true, &avail, 0), None);
}

#[test]
fn earlier_files_come_first() {
    let avail = [9, 8, 1, 2];
    let groups = || vec![vec![0, 1], vec![2, 3]];
    // Piece 2 is rarer, but the first file wins.
    assert_eq!(pick(groups(), |_| true, &avail, 0), Some(1));
    // The peer has nothing from the first file, so it serves the next one.
    assert_eq!(pick(groups(), |p| p >= 2, &avail, 0), Some(2));
}

#[test]
fn ties_spread_over_every_piece_and_repeat_for_one_salt() {
    let avail = [2; 8];
    let all = || vec![(0..8).collect::<Vec<_>>()];
    let mut seen = [false; 8];
    for salt in 0..400 {
        seen[pick(all(), |_| true, &avail, salt).unwrap()] = true;
    }
    assert!(seen.iter().all(|s| *s), "{seen:?}");
    assert_eq!(
        pick(all(), |_| true, &avail, 7),
        pick(all(), |_| true, &avail, 7)
    );
}

#[test]
fn pieces_without_a_count_count_as_common() {
    assert_eq!(pick(vec![vec![0, 1]], |_| true, &[3], 0), Some(0));
}
