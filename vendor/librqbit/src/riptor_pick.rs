// RIPTOR PATCH: rarest-first piece choice, kept free of librqbit types so it can be tested.

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// The piece fewest peers have, from the first group (file) with one `has` allows.
/// Ties are broken by `salt`, so peers don't all start at the same piece.
pub fn pick_rarest<T: Copy, G: IntoIterator<Item = T>>(
    groups: impl IntoIterator<Item = G>,
    index: impl Fn(T) -> usize,
    has: impl Fn(T) -> bool,
    availability: &[u16],
    salt: u64,
) -> Option<T> {
    for group in groups {
        let mut best: Option<(u16, T)> = None;
        let mut ties = 0u64;
        for piece in group {
            if !has(piece) {
                continue;
            }
            let seen = availability.get(index(piece)).copied().unwrap_or(u16::MAX);
            match best {
                Some((b, _)) if seen > b => {}
                Some((b, _)) if seen == b => {
                    ties += 1;
                    if mix(salt ^ ties).is_multiple_of(ties) {
                        best = Some((seen, piece));
                    }
                }
                _ => {
                    best = Some((seen, piece));
                    ties = 1;
                }
            }
        }
        if let Some((_, piece)) = best {
            return Some(piece);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::pick_rarest;

    #[test]
    fn takes_the_rarest_the_peer_has() {
        let avail = [5, 1, 3, 2, 9];
        let pick = |has: &dyn Fn(usize) -> bool| pick_rarest([vec![0usize, 1, 2, 3, 4]], |p| p, has, &avail, 0);
        assert_eq!(pick(&|_| true), Some(1));
        assert_eq!(pick(&|p| p != 1), Some(3));
        assert_eq!(pick(&|p| p == 0 || p == 4), Some(0));
        assert_eq!(pick(&|_| false), None);
    }

    #[test]
    fn earlier_files_come_first() {
        let avail = [9, 8, 1, 2];
        let groups = [vec![0usize, 1], vec![2, 3]];
        assert_eq!(pick_rarest(groups.clone(), |p| p, |_| true, &avail, 0), Some(1));
        // The peer has nothing from the first file, so the next one is used.
        assert_eq!(pick_rarest(groups, |p| p, |p| p >= 2, &avail, 0), Some(2));
    }

    #[test]
    fn ties_follow_the_salt_and_cover_every_piece() {
        let avail = [2; 8];
        let mut seen = [false; 8];
        for salt in 0..400 {
            seen[pick_rarest([(0..8usize).collect::<Vec<_>>()], |p| p, |_| true, &avail, salt).unwrap()] = true;
        }
        assert!(seen.iter().all(|s| *s), "{seen:?}");
        let again = |s| pick_rarest([(0..8usize).collect::<Vec<_>>()], |p| p, |_| true, &avail, s);
        assert_eq!(again(7), again(7));
    }

    #[test]
    fn pieces_without_a_count_count_as_common() {
        assert_eq!(pick_rarest([vec![0usize, 1]], |p| p, |_| true, &[3], 0), Some(0));
    }
}
