//! MD4 ("mdfour") port of `Quake/mdfour.c` (Tridgell's Samba implementation).
//!
//! One-shot API only: the C `mdfour_update` unconditionally runs the tail
//! padding at the end of every call (Spike's comment marks where the streaming
//! variant was buggy and got disabled), so this port doesn't pretend to
//! stream either.
//!
//! `block_checksum` folds the 128-bit digest to 32 bits by xoring the four
//! little-endian words — that folded value is the `csprogsvers/%x.dat`
//! filename key (`qcvm->progshash`) and must be preserved exactly.
//!
//! The bit-length counter is 32 bits wide (`uint32 b = totalN * 8`), wrapping
//! for inputs over 512 MB exactly as the C does.

struct MdFour {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    total_n: u32,
}

#[inline]
fn f(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | (!x & z)
}

#[inline]
fn g(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | (x & z) | (y & z)
}

#[inline]
fn h(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}

impl MdFour {
    fn begin() -> Self {
        MdFour {
            a: 0x67452301,
            b: 0xefcdab89,
            c: 0x98badcfe,
            d: 0x10325476,
            total_n: 0,
        }
    }

    fn round64(&mut self, x: &[u32; 16]) {
        let (mut a, mut b, mut c, mut d) = (self.a, self.b, self.c, self.d);
        let (aa, bb, cc, dd) = (a, b, c, d);

        macro_rules! round1 {
            ($a:ident, $b:ident, $c:ident, $d:ident, $k:expr, $s:expr) => {
                $a = $a
                    .wrapping_add(f($b, $c, $d))
                    .wrapping_add(x[$k])
                    .rotate_left($s)
            };
        }
        macro_rules! round2 {
            ($a:ident, $b:ident, $c:ident, $d:ident, $k:expr, $s:expr) => {
                $a = $a
                    .wrapping_add(g($b, $c, $d))
                    .wrapping_add(x[$k])
                    .wrapping_add(0x5A827999)
                    .rotate_left($s)
            };
        }
        macro_rules! round3 {
            ($a:ident, $b:ident, $c:ident, $d:ident, $k:expr, $s:expr) => {
                $a = $a
                    .wrapping_add(h($b, $c, $d))
                    .wrapping_add(x[$k])
                    .wrapping_add(0x6ED9EBA1)
                    .rotate_left($s)
            };
        }

        round1!(a, b, c, d, 0, 3);
        round1!(d, a, b, c, 1, 7);
        round1!(c, d, a, b, 2, 11);
        round1!(b, c, d, a, 3, 19);
        round1!(a, b, c, d, 4, 3);
        round1!(d, a, b, c, 5, 7);
        round1!(c, d, a, b, 6, 11);
        round1!(b, c, d, a, 7, 19);
        round1!(a, b, c, d, 8, 3);
        round1!(d, a, b, c, 9, 7);
        round1!(c, d, a, b, 10, 11);
        round1!(b, c, d, a, 11, 19);
        round1!(a, b, c, d, 12, 3);
        round1!(d, a, b, c, 13, 7);
        round1!(c, d, a, b, 14, 11);
        round1!(b, c, d, a, 15, 19);

        round2!(a, b, c, d, 0, 3);
        round2!(d, a, b, c, 4, 5);
        round2!(c, d, a, b, 8, 9);
        round2!(b, c, d, a, 12, 13);
        round2!(a, b, c, d, 1, 3);
        round2!(d, a, b, c, 5, 5);
        round2!(c, d, a, b, 9, 9);
        round2!(b, c, d, a, 13, 13);
        round2!(a, b, c, d, 2, 3);
        round2!(d, a, b, c, 6, 5);
        round2!(c, d, a, b, 10, 9);
        round2!(b, c, d, a, 14, 13);
        round2!(a, b, c, d, 3, 3);
        round2!(d, a, b, c, 7, 5);
        round2!(c, d, a, b, 11, 9);
        round2!(b, c, d, a, 15, 13);

        round3!(a, b, c, d, 0, 3);
        round3!(d, a, b, c, 8, 9);
        round3!(c, d, a, b, 4, 11);
        round3!(b, c, d, a, 12, 15);
        round3!(a, b, c, d, 2, 3);
        round3!(d, a, b, c, 10, 9);
        round3!(c, d, a, b, 6, 11);
        round3!(b, c, d, a, 14, 15);
        round3!(a, b, c, d, 1, 3);
        round3!(d, a, b, c, 9, 9);
        round3!(c, d, a, b, 5, 11);
        round3!(b, c, d, a, 13, 15);
        round3!(a, b, c, d, 3, 3);
        round3!(d, a, b, c, 11, 9);
        round3!(c, d, a, b, 7, 11);
        round3!(b, c, d, a, 15, 15);

        self.a = a.wrapping_add(aa);
        self.b = b.wrapping_add(bb);
        self.c = c.wrapping_add(cc);
        self.d = d.wrapping_add(dd);
    }

    fn tail(&mut self, tail: &[u8]) {
        let n = tail.len();
        debug_assert!(n < 64);

        self.total_n = self.total_n.wrapping_add(n as u32);
        let b = self.total_n.wrapping_mul(8);

        let mut buf = [0u8; 128];
        buf[..n].copy_from_slice(tail);
        buf[n] = 0x80;

        if n <= 55 {
            buf[56..60].copy_from_slice(&b.to_le_bytes());
            self.round64(&load64(&buf[..64]));
        } else {
            buf[120..124].copy_from_slice(&b.to_le_bytes());
            self.round64(&load64(&buf[..64]));
            self.round64(&load64(&buf[64..128]));
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        while input.len() >= 64 {
            self.round64(&load64(&input[..64]));
            input = &input[64..];
            self.total_n = self.total_n.wrapping_add(64);
        }
        self.tail(input);
    }

    fn result(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&self.a.to_le_bytes());
        out[4..8].copy_from_slice(&self.b.to_le_bytes());
        out[8..12].copy_from_slice(&self.c.to_le_bytes());
        out[12..16].copy_from_slice(&self.d.to_le_bytes());
        out
    }
}

fn load64(chunk: &[u8]) -> [u32; 16] {
    let mut m = [0u32; 16];
    for (i, word) in chunk.chunks_exact(4).enumerate() {
        m[i] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
    }
    m
}

/// One-shot MD4 digest (C `mdfour`/`Com_BlockFullChecksum`).
pub fn mdfour(input: &[u8]) -> [u8; 16] {
    let mut md = MdFour::begin();
    md.update(input);
    md.result()
}

/// Folded 32-bit checksum (C `Com_BlockChecksum`): xor of the four
/// little-endian digest words. // COMPAT: the fold is the csprogsvers/%x.dat key.
pub fn block_checksum(input: &[u8]) -> u32 {
    let digest = mdfour(input);
    let d0 = u32::from_le_bytes(digest[0..4].try_into().unwrap());
    let d1 = u32::from_le_bytes(digest[4..8].try_into().unwrap());
    let d2 = u32::from_le_bytes(digest[8..12].try_into().unwrap());
    let d3 = u32::from_le_bytes(digest[12..16].try_into().unwrap());
    d0 ^ d1 ^ d2 ^ d3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(digest: &[u8; 16]) -> String {
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    // RFC 1320 appendix A.5 test vectors
    #[test]
    fn rfc1320_vectors() {
        assert_eq!(hex(&mdfour(b"")), "31d6cfe0d16ae931b73c59d7e0c089c0");
        assert_eq!(hex(&mdfour(b"a")), "bde52cb31de33e46245e05fbdbd6fb24");
        assert_eq!(hex(&mdfour(b"abc")), "a448017aaf21d8525fc10ae87aa6729d");
        assert_eq!(
            hex(&mdfour(b"message digest")),
            "d9130a8164549fe818874806e1c7014b"
        );
        assert_eq!(
            hex(&mdfour(b"abcdefghijklmnopqrstuvwxyz")),
            "d79e1c308aa5bbcdeea8ed63df412da9"
        );
        assert_eq!(
            hex(&mdfour(
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
            )),
            "043f8582f241db351ce627e153e7f0e4"
        );
        assert_eq!(
            hex(&mdfour(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            )),
            "e33b4ddc9c38f2199c3e7b164fcc0536"
        );
    }

    #[test]
    fn fold_is_xor_of_le_words() {
        let digest = mdfour(b"abc");
        let expect = u32::from_le_bytes(digest[0..4].try_into().unwrap())
            ^ u32::from_le_bytes(digest[4..8].try_into().unwrap())
            ^ u32::from_le_bytes(digest[8..12].try_into().unwrap())
            ^ u32::from_le_bytes(digest[12..16].try_into().unwrap());
        assert_eq!(block_checksum(b"abc"), expect);
    }

    #[test]
    fn boundary_lengths() {
        // exercise the n <= 55 / n > 55 tail split and multi-block updates
        for len in [54usize, 55, 56, 63, 64, 65, 119, 120, 127, 128, 129] {
            let data = vec![0xa5u8; len];
            // just ensure it runs and is stable; exact values pinned by the
            // differential suite in quake-ctest
            assert_eq!(mdfour(&data), mdfour(&data));
        }
    }
}
