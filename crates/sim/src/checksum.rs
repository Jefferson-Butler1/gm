use std::hash::{Hash, Hasher};

/// FNV-1a over the state's `Hash` output. std's default `Hasher` integer writes use native
/// endianness and `usize` width, so both are pinned here to keep checksums comparable
/// across machines (CI cross-checks `x86_64` against `aarch64`).
pub fn of(value: &impl Hash) -> u64 {
    let mut hasher = Fnv1a(0xCBF2_9CE4_8422_2325);
    value.hash(&mut hasher);
    hasher.finish()
}

struct Fnv1a(u64);

impl Hasher for Fnv1a {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3);
        }
    }

    // Signed writes forward to these by default.
    fn write_u16(&mut self, i: u16) {
        self.write(&i.to_le_bytes());
    }
    fn write_u32(&mut self, i: u32) {
        self.write(&i.to_le_bytes());
    }
    fn write_u64(&mut self, i: u64) {
        self.write(&i.to_le_bytes());
    }
    fn write_u128(&mut self, i: u128) {
        self.write(&i.to_le_bytes());
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(u64::try_from(i).unwrap_or(u64::MAX));
    }
}
