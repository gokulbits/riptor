//! Unflushed byte ranges of one piece. Memory follows what was received, not
//! the piece size. Newer writes win on overlap.

use std::collections::BTreeMap;

#[derive(Default, Debug)]
pub struct Segments {
    map: BTreeMap<u64, Vec<u8>>,
    bytes: usize,
}

impl Segments {
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Returns the change in buffered bytes.
    pub fn insert(&mut self, start: u64, data: Vec<u8>) -> isize {
        let before = self.bytes as isize;
        let end = start + data.len() as u64;
        if data.is_empty() {
            return 0;
        }
        let overlapping: Vec<u64> = self
            .map
            .range(..end)
            .rev()
            .take_while(|(k, v)| **k + v.len() as u64 > start)
            .map(|(k, _)| *k)
            .collect();
        for k in overlapping {
            let v = self.map.remove(&k).expect("key collected above");
            self.bytes -= v.len();
            let v_end = k + v.len() as u64;
            if k < start {
                let head = v[..(start - k) as usize].to_vec();
                self.bytes += head.len();
                self.map.insert(k, head);
            }
            if v_end > end {
                let tail = v[(end - k) as usize..].to_vec();
                self.bytes += tail.len();
                self.map.insert(end, tail);
            }
        }
        self.bytes += data.len();
        self.map.insert(start, data);
        self.bytes as isize - before
    }

    pub fn covers(&self, start: u64, len: u64) -> bool {
        let end = start + len;
        let mut pos = start;
        if let Some((k, v)) = self.map.range(..=start).next_back() {
            if k + v.len() as u64 > start {
                pos = k + v.len() as u64;
            }
        }
        while pos < end {
            match self.map.get(&pos) {
                Some(v) => pos += v.len() as u64,
                None => return false,
            }
        }
        true
    }

    pub fn overlay(&self, start: u64, buf: &mut [u8]) {
        let end = start + buf.len() as u64;
        let first = self
            .map
            .range(..=start)
            .next_back()
            .map(|(k, _)| *k)
            .unwrap_or(start);
        for (k, v) in self.map.range(first..end) {
            let s = start.max(*k);
            let e = end.min(k + v.len() as u64);
            if s < e {
                buf[(s - start) as usize..(e - start) as usize]
                    .copy_from_slice(&v[(s - k) as usize..(e - k) as usize]);
            }
        }
    }

    /// Merged, in order.
    pub fn runs(&self) -> Vec<(u64, Vec<u8>)> {
        let mut out: Vec<(u64, Vec<u8>)> = Vec::new();
        for (k, v) in &self.map {
            match out.last_mut() {
                Some((s, buf)) if *s + buf.len() as u64 == *k => buf.extend_from_slice(v),
                _ => out.push((*k, v.clone())),
            }
        }
        out
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn coalesces_runs() {
        let mut s = Segments::default();
        s.insert(0, vec![1; 4]);
        s.insert(4, vec![2; 4]);
        s.insert(10, vec![3; 2]);
        assert_eq!(
            s.runs(),
            vec![(0, vec![1, 1, 1, 1, 2, 2, 2, 2]), (10, vec![3, 3])]
        );
        assert!(s.covers(0, 8));
        assert!(s.covers(2, 5));
        assert!(!s.covers(0, 9));
        assert!(!s.covers(9, 2));
        assert!(s.covers(10, 2));
    }

    #[test]
    fn newer_writes_win() {
        let mut s = Segments::default();
        s.insert(0, vec![1; 10]);
        assert_eq!(s.insert(3, vec![9; 4]), 0);
        let mut buf = [0u8; 10];
        s.overlay(0, &mut buf);
        assert_eq!(buf, [1, 1, 1, 9, 9, 9, 9, 1, 1, 1]);
        assert_eq!(s.bytes(), 10);
    }

    proptest! {
        #[test]
        fn matches_flat_model(ops in proptest::collection::vec((0u64..200, 1usize..40, any::<u8>()), 1..60)) {
            const N: usize = 256;
            let mut model = [None::<u8>; N];
            let mut s = Segments::default();
            for (start, len, val) in ops {
                let len = len.min(N - start as usize);
                s.insert(start, vec![val; len]);
                for slot in model.iter_mut().skip(start as usize).take(len) {
                    *slot = Some(val);
                }
            }
            let mut buf = [0u8; N];
            s.overlay(0, &mut buf);
            for i in 0..N {
                if let Some(v) = model[i] { prop_assert_eq!(buf[i], v); }
                prop_assert_eq!(s.covers(i as u64, 1), model[i].is_some());
            }
            let buffered = model.iter().filter(|m| m.is_some()).count();
            prop_assert_eq!(s.bytes(), buffered);
            let run_bytes: usize = s.runs().iter().map(|(_, v)| v.len()).sum();
            prop_assert_eq!(run_bytes, buffered);
        }
    }
}
