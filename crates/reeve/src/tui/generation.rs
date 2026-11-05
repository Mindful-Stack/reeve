//! Generation counters: a timer or reply carries the generation current when it was issued, and
//! is dropped when its key has moved on. Values start at 1; 0 is never valid.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Gen(u64);
impl Gen {
    pub fn bump(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }
    pub fn is(&self, g: u64) -> bool {
        g != 0 && self.0 == g
    }
    pub fn current(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_counter() {
        let mut g = Gen::default();
        assert!(!g.is(0));
        let one = g.bump();
        assert!(g.is(one));
        g.bump();
        assert!(!g.is(one));
    }
}
