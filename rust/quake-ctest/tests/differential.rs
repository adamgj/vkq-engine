//! Differential tests: Rust ports vs the original C (compiled as c_ref_*).

use proptest::prelude::*;
use quake_ctest::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn crc_block_matches(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        prop_assert_eq!(quake_util::crc::crc_block(&data), c_crc_block(&data));
    }

    #[test]
    fn crc_incremental_matches(data in proptest::collection::vec(any::<u8>(), 0..512)) {
        let mut rust_crc = quake_util::crc::crc_init();
        let mut c_crc = c_crc_init();
        prop_assert_eq!(rust_crc, c_crc);
        for &b in &data {
            quake_util::crc::crc_process_byte(&mut rust_crc, b);
            c_crc_process_byte(&mut c_crc, b);
            prop_assert_eq!(rust_crc, c_crc);
        }
        prop_assert_eq!(quake_util::crc::crc_value(rust_crc), c_crc_value(c_crc));
    }

    #[test]
    fn mdfour_digest_matches(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        prop_assert_eq!(quake_util::mdfour::mdfour(&data), c_block_full_checksum(&data));
    }

    #[test]
    fn mdfour_fold_matches(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        prop_assert_eq!(quake_util::mdfour::block_checksum(&data), c_block_checksum(&data));
    }

    #[test]
    fn strlcpy_matches(
        dst in proptest::collection::vec(any::<u8>(), 1..96),
        src in proptest::collection::vec(1u8..=255, 0..96),
        siz_frac in 0.0f64..=1.0,
    ) {
        let siz = ((dst.len() as f64) * siz_frac) as usize;
        let (c_ret, c_buf) = c_strlcpy(&dst, &src, siz);
        let mut rust_buf = dst.clone();
        let rust_ret = quake_util::strl::strlcpy(&mut rust_buf[..siz], &src);
        prop_assert_eq!(rust_ret, c_ret);
        prop_assert_eq!(rust_buf, c_buf);
    }

    #[test]
    fn strlcat_matches(
        dst in proptest::collection::vec(any::<u8>(), 1..96),
        src in proptest::collection::vec(1u8..=255, 0..96),
        siz_frac in 0.0f64..=1.0,
    ) {
        let siz = ((dst.len() as f64) * siz_frac) as usize;
        let (c_ret, c_buf) = c_strlcat(&dst, &src, siz);
        let mut rust_buf = dst.clone();
        let rust_ret = quake_util::strl::strlcat(&mut rust_buf[..siz], &src);
        prop_assert_eq!(rust_ret, c_ret);
        prop_assert_eq!(rust_buf, c_buf);
    }
}

// mdfour tail-boundary lengths (n<=55 / n>55 split, multi-block) pinned
// against the C explicitly, since random sampling may miss exact boundaries
#[test]
fn mdfour_boundary_lengths_match() {
    for len in [
        0usize, 1, 54, 55, 56, 57, 63, 64, 65, 119, 120, 121, 127, 128, 129, 4096,
    ] {
        let data: Vec<u8> = (0..len).map(|i| (i * 7 + 13) as u8).collect();
        assert_eq!(
            quake_util::mdfour::mdfour(&data),
            c_block_full_checksum(&data),
            "digest mismatch at len {len}"
        );
        assert_eq!(
            quake_util::mdfour::block_checksum(&data),
            c_block_checksum(&data),
            "fold mismatch at len {len}"
        );
    }
}

// shim-level checks: the exact extern "C" entry points the engine will link
#[test]
fn capi_shims_match_c() {
    let data = b"quake-ctest shim probe";
    // SAFETY: valid slice pointers/lengths; outbuf is 16 writable bytes
    unsafe {
        assert_eq!(
            quake_rs::crc::CRC_Block(data.as_ptr(), data.len() as i32),
            c_crc_block(data)
        );
        let mut crc = 0u16;
        quake_rs::crc::CRC_Init(&mut crc);
        assert_eq!(crc, c_crc_init());

        assert_eq!(
            quake_rs::mdfour::Com_BlockChecksum(data.as_ptr() as *mut _, data.len() as i32),
            c_block_checksum(data)
        );
        let mut out = [0u8; 16];
        quake_rs::mdfour::Com_BlockFullChecksum(
            data.as_ptr() as *mut _,
            data.len() as i32,
            out.as_mut_ptr(),
        );
        assert_eq!(out, c_block_full_checksum(data));

        let mut dst = [0u8; 8];
        let ret = quake_rs::strl::q_strlcpy(
            dst.as_mut_ptr() as *mut _,
            c"abcdefghij".as_ptr(),
            dst.len(),
        );
        let (c_ret, c_buf) = c_strlcpy(&[0u8; 8], b"abcdefghij", 8);
        assert_eq!(ret, c_ret);
        assert_eq!(&dst[..], &c_buf[..]);

        let mut dst2 = *b"ab\0xxxxx";
        let ret =
            quake_rs::strl::q_strlcat(dst2.as_mut_ptr() as *mut _, c"cdefg".as_ptr(), dst2.len());
        let (c_ret2, c_buf2) = c_strlcat(b"ab\0xxxxx", b"cdefg", 8);
        assert_eq!(ret, c_ret2);
        assert_eq!(&dst2[..], &c_buf2[..]);
    }
}
