//! CRC-16/CCITT (XMODEM) port of `Quake/crc.c`.
//!
//! Non-reflected, polynomial 0x1021, init 0xFFFF, final xor 0x0000. The final
//! xor is zero, so `crc_value` is an identity function and `crc_block` returns
//! the raw running value — both quirks are observable (progs CRC, texture CRC)
//! and preserved.

const CRC_INIT_VALUE: u16 = 0xffff;
const CRC_XOR_VALUE: u16 = 0x0000;

const fn build_table() -> [u16; 256] {
    let mut table = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 0x8000 != 0 {
                (c << 1) ^ 0x1021
            } else {
                c << 1
            };
            bit += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

pub(crate) const CRC_TABLE: [u16; 256] = build_table();

pub fn crc_init() -> u16 {
    CRC_INIT_VALUE
}

pub fn crc_process_byte(crcvalue: &mut u16, data: u8) {
    *crcvalue = (*crcvalue << 8) ^ CRC_TABLE[((*crcvalue >> 8) ^ data as u16) as usize];
}

pub fn crc_value(crcvalue: u16) -> u16 {
    crcvalue ^ CRC_XOR_VALUE
}

pub fn crc_block(data: &[u8]) -> u16 {
    let mut crc = CRC_INIT_VALUE;
    for &b in data {
        crc = (crc << 8) ^ CRC_TABLE[((crc >> 8) ^ b as u16) as usize];
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_c_table() {
        // spot values transcribed from the crctable[] literal in Quake/crc.c
        assert_eq!(CRC_TABLE[0], 0x0000);
        assert_eq!(CRC_TABLE[1], 0x1021);
        assert_eq!(CRC_TABLE[8], 0x8108);
        assert_eq!(CRC_TABLE[119], 0x0e70);
        assert_eq!(CRC_TABLE[127], 0x8f78);
        assert_eq!(CRC_TABLE[128], 0x9188);
        assert_eq!(CRC_TABLE[255], 0x1ef0);
    }

    #[test]
    fn known_vectors() {
        // standard CRC-16/CCITT-FALSE check value for "123456789"
        assert_eq!(crc_block(b"123456789"), 0x29b1);
        assert_eq!(crc_block(b""), 0xffff);
    }

    #[test]
    fn process_byte_matches_block() {
        let data = b"the quick brown fox";
        let mut crc = crc_init();
        for &b in data {
            crc_process_byte(&mut crc, b);
        }
        assert_eq!(crc_value(crc), crc_block(data));
    }
}
