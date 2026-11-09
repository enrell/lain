/// A piece set in wire order: piece 0 is the high bit of byte 0 (BEP 3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bitfield(pub Vec<u8>);

impl Bitfield {
    /// An empty set for n pieces.
    pub fn new(n: usize) -> Bitfield {
        Bitfield(vec![0; n.div_ceil(8)])
    }
    pub fn from_bytes(b: &[u8]) -> Bitfield {
        Bitfield(b.to_vec())
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn len_bytes(&self) -> usize {
        self.0.len()
    }
    pub fn has(&self, i: usize) -> bool {
        self.0.get(i / 8).is_some_and(|b| b & (0x80 >> (i % 8)) != 0)
    }
    pub fn set(&mut self, i: usize) {
        if let Some(b) = self.0.get_mut(i / 8) {
            *b |= 0x80 >> (i % 8);
        }
    }
    pub fn clear(&mut self, i: usize) {
        if let Some(b) = self.0.get_mut(i / 8) {
            *b &= !(0x80 >> (i % 8));
        }
    }
    /// Pieces set among the first n.
    pub fn count(&self, n: usize) -> usize {
        (0..n).filter(|&i| self.has(i)).count()
    }
    /// Whether a peer's bitfield fits n pieces: right length and no
    /// spare bits set (BEP 3 says drop such peers).
    pub fn valid(&self, n: usize) -> bool {
        self.0.len() == n.div_ceil(8) && (n..self.0.len() * 8).all(|i| !self.has(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_in_wire_order() {
        let mut b = Bitfield::new(10);
        assert_eq!(b.len_bytes(), 2);
        b.set(0);
        b.set(9);
        b.set(99);
        assert_eq!(b.0, vec![0x80, 0x40]);
        assert!(b.has(9) && !b.has(1) && !b.has(1000));
        assert_eq!(b.count(10), 2);
        assert!(b.valid(10));
        b.0[1] |= 0x01;
        assert!(!b.valid(10), "spare bits set");
        assert!(!Bitfield::new(9).valid(17));
        b.clear(0);
        assert!(!b.has(0));
    }
}
