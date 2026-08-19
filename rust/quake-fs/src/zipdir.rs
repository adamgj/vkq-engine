//! Minimal zip (.kpf) reader mirroring the vendored `Quake/miniz.c` reader in
//! the exact configuration the engine uses: `mz_zip_reader_init` over a whole
//! file (`MZ_ZIP_TYPE_USER`, flags 0) in `LOC_LoadFile`, then
//! `mz_zip_reader_extract_file_to_heap` with flags 0 (case-insensitive
//! full-path locate via the sorted central dir; stored + deflate only; CRC32
//! always verified). `inflate_embedded` mirrors the raw `tinfl_decompress`
//! call that unpacks the embedded vkquake.pak in `COM_AddGameDirectoryRoot`.
//!
//! Deliberately replicated miniz quirks:
//! - the EOCD scan walks backwards in 4096-byte chunks overlapping by 3 and
//!   gives up only after fully scanning a chunk that starts `>= 65535 + 22`
//!   bytes from the end;
//! - bytes between the end of the central dir and the EOCD are treated as an
//!   archive *prefix*: `m_archive_size` shrinks by that amount, but reads stay
//!   absolute (the engine's read callback ignores `m_file_archive_start_ofs`);
//! - the open-time zip64 extra-field probe that re-reads from the file uses an
//!   offset relative to the central dir *start*, not the current entry
//!   (miniz bug, kept);
//! - the stored-entry size consistency check at open fires only when the
//!   32-bit read at the method offset is zero, i.e. method *and* DOS time;
//! - directory entries (trailing '/' or DOS dir attribute) and entries with
//!   zero compressed size "extract" successfully without touching the output;
//! - stored extraction reads `uncomp_size` bytes from the file, not
//!   `comp_size`, and 64-bit offset/size arithmetic wraps like C `mz_uint64`.
//!
//! Known divergences:
//! - the untouched-output cases above return zero-filled memory where miniz
//!   returns uninitialized malloc memory (accept/reject unaffected);
//! - `AllocFailed` comes from the Rust allocator's `try_reserve` instead of
//!   `malloc`, so the failure threshold for absurd claimed sizes can differ
//!   (both fail near `u64::MAX`, both succeed for ordinary archives);
//! - inflation is `miniz_oxide`'s port of tinfl rather than the vendored
//!   copy; the status contract (Done / needs-input / has-output / failed) is
//!   identical and the ADR-012 corpus gate arbitrates residual drift.

use miniz_oxide::inflate::core::inflate_flags::{
    TINFL_FLAG_HAS_MORE_INPUT, TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
};
use miniz_oxide::inflate::core::{decompress, DecompressorOxide};
use miniz_oxide::inflate::TINFLStatus;

const MZ_ZIP_END_OF_CENTRAL_DIR_HEADER_SIG: u32 = 0x06054b50;
const MZ_ZIP_CENTRAL_DIR_HEADER_SIG: u32 = 0x02014b50;
const MZ_ZIP_LOCAL_DIR_HEADER_SIG: u32 = 0x04034b50;
const MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIG: u32 = 0x06064b50;
const MZ_ZIP64_END_OF_CENTRAL_DIR_LOCATOR_SIG: u32 = 0x07064b50;
const MZ_ZIP_LOCAL_DIR_HEADER_SIZE: u64 = 30;
const MZ_ZIP_CENTRAL_DIR_HEADER_SIZE: u32 = 46;
const MZ_ZIP_END_OF_CENTRAL_DIR_HEADER_SIZE: u64 = 22;
const MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIZE: u64 = 56;
const MZ_ZIP64_END_OF_CENTRAL_DIR_LOCATOR_SIZE: u64 = 20;
const MZ_ZIP64_EXTENDED_INFORMATION_FIELD_HEADER_ID: u16 = 0x0001;
const MZ_ZIP_DOS_DIR_ATTRIBUTE_BITFLAG: u32 = 0x10;
const MZ_ZIP_GENERAL_PURPOSE_BIT_FLAG_IS_ENCRYPTED: u16 = 1;
const MZ_ZIP_GENERAL_PURPOSE_BIT_FLAG_COMPRESSED_PATCH_FLAG: u16 = 32;
const MZ_ZIP_GENERAL_PURPOSE_BIT_FLAG_USES_STRONG_ENCRYPTION: u16 = 64;
const MZ_ZIP_GENERAL_PURPOSE_BIT_FLAG_LOCAL_DIR_IS_MASKED: u16 = 8192;
const MZ_ZIP_MAX_IO_BUF_SIZE: u64 = 64 * 1024;
const MZ_DEFLATED: u16 = 8;

/// `mz_zip_error` values reachable through the paths this module mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipError {
    NotAnArchive,
    FailedFindingCentralDir,
    UnsupportedMultidisk,
    TooManyFiles,
    UnsupportedCdirSize,
    InvalidHeaderOrCorrupted,
    FileReadFailed,
    UnsupportedEncryption,
    UnsupportedMethod,
    AllocFailed,
    FileNotFound,
    InvalidParameter,
    DecompressionFailed,
    UnexpectedDecompressedSize,
    CrcCheckFailed,
}

fn le16(b: &[u8], ofs: usize) -> u16 {
    u16::from_le_bytes([b[ofs], b[ofs + 1]])
}

fn le32(b: &[u8], ofs: usize) -> u32 {
    u32::from_le_bytes([b[ofs], b[ofs + 1], b[ofs + 2], b[ofs + 3]])
}

fn le64(b: &[u8], ofs: usize) -> u64 {
    u64::from_le_bytes([
        b[ofs],
        b[ofs + 1],
        b[ofs + 2],
        b[ofs + 3],
        b[ofs + 4],
        b[ofs + 5],
        b[ofs + 6],
        b[ofs + 7],
    ])
}

/// The engine's `mz_zip_file_read_func`: seek + read succeeds only in full.
fn file_read(data: &[u8], ofs: u64, n: u64) -> Option<&[u8]> {
    let end = ofs.checked_add(n)?;
    if end > data.len() as u64 {
        return None;
    }
    Some(&data[usize::try_from(ofs).ok()?..usize::try_from(end).ok()?])
}

/// `mz_crc32` (standard CRC-32, bitwise).
fn mz_crc32(buf: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in buf {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn tolower(c: u8) -> u8 {
    // MZ_TOLOWER: ASCII only
    if c.is_ascii_uppercase() {
        c + (b'a' - b'A')
    } else {
        c
    }
}

/// `mz_zip_reader_locate_header_sig` for the EOCD record: backwards scan in
/// 4096-byte chunks overlapping by 3, giving up after a whole failed chunk
/// that starts `>= 65535 + record_size` bytes from the end.
fn locate_eocd(data: &[u8]) -> Option<u64> {
    const RECORD_SIZE: u64 = MZ_ZIP_END_OF_CENTRAL_DIR_HEADER_SIZE;
    const BUF: u64 = 4096;
    let size = data.len() as u64;
    if size < RECORD_SIZE {
        return None;
    }
    let mut cur = size.saturating_sub(BUF);
    loop {
        let n = usize::try_from(BUF.min(size - cur)).ok()?;
        let chunk = &data[usize::try_from(cur).ok()?..][..n];
        if n >= 4 {
            for i in (0..=n - 4).rev() {
                if le32(chunk, i) == MZ_ZIP_END_OF_CENTRAL_DIR_HEADER_SIG
                    && size - (cur + i as u64) >= RECORD_SIZE
                {
                    return Some(cur + i as u64);
                }
            }
        }
        if cur == 0 || size - cur >= u64::from(u16::MAX) + RECORD_SIZE {
            return None;
        }
        cur = cur.saturating_sub(BUF - 3);
    }
}

fn entry_filename(cdir: &[u8], entry_ofs: u32) -> &[u8] {
    let p = entry_ofs as usize;
    let len = le16(cdir, p + 28) as usize;
    &cdir[p + 46..p + 46 + len]
}

/// `mz_zip_reader_filename_less` (lowercased lexicographic; prefix ties go to
/// the shorter name).
fn filename_less(cdir: &[u8], l_ofs: u32, r_ofs: u32) -> bool {
    let l = entry_filename(cdir, l_ofs);
    let r = entry_filename(cdir, r_ofs);
    for (&a, &b) in l.iter().zip(r.iter()) {
        let (a, b) = (tolower(a), tolower(b));
        if a != b {
            return a < b;
        }
    }
    l.len() < r.len()
}

/// `mz_zip_filename_compare`; the length difference is the C's
/// `(int)(l_len - r_len)` over `mz_uint` operands.
fn filename_compare(cdir: &[u8], entry_ofs: u32, name: &[u8]) -> i32 {
    let fname = entry_filename(cdir, entry_ofs);
    for (&lc, &rc) in fname.iter().zip(name.iter()) {
        let (l, r) = (tolower(lc), tolower(rc));
        if l != r {
            return i32::from(l) - i32::from(r);
        }
    }
    (fname.len() as u32).wrapping_sub(name.len() as u32) as i32
}

/// `mz_zip_reader_sort_central_dir_offsets_by_filename`: the exact heap sort,
/// so duplicate-name archives resolve to the same entry the C picks.
fn sort_central_dir_offsets_by_filename(cdir: &[u8], entry_ofs: &[u32], indices: &mut [u32]) {
    let size = indices.len();
    if size <= 1 {
        return;
    }
    let less = |l: u32, r: u32| filename_less(cdir, entry_ofs[l as usize], entry_ofs[r as usize]);
    let mut start = (size - 2) >> 1;
    loop {
        let mut root = start;
        loop {
            let mut child = (root << 1) + 1;
            if child >= size {
                break;
            }
            if child + 1 < size && less(indices[child], indices[child + 1]) {
                child += 1;
            }
            if !less(indices[root], indices[child]) {
                break;
            }
            indices.swap(root, child);
            root = child;
        }
        if start == 0 {
            break;
        }
        start -= 1;
    }
    let mut end = size - 1;
    while end > 0 {
        indices.swap(end, 0);
        let mut root = 0;
        loop {
            let mut child = (root << 1) + 1;
            if child >= end {
                break;
            }
            if child + 1 < end && less(indices[child], indices[child + 1]) {
                child += 1;
            }
            if !less(indices[root], indices[child]) {
                break;
            }
            indices.swap(root, child);
            root = child;
        }
        end -= 1;
    }
}

/// The extraction-relevant subset of `mz_zip_archive_file_stat`.
struct FileStat {
    bit_flag: u16,
    method: u16,
    crc32: u32,
    comp_size: u64,
    uncomp_size: u64,
    local_header_ofs: u64,
    is_directory: bool,
}

pub struct ZipArchive<'a> {
    data: &'a [u8],
    /// `m_archive_size` after the trailing-prefix adjustment.
    archive_size: u64,
    /// `m_central_dir` (a slice of `data`, validated at open).
    cdir: &'a [u8],
    /// `m_central_dir_offsets`.
    entry_ofs: Vec<u32>,
    /// `m_sorted_central_dir_offsets`.
    sorted: Vec<u32>,
}

impl<'a> ZipArchive<'a> {
    /// `mz_zip_reader_init(&archive, data.len(), 0)` +
    /// `mz_zip_reader_read_central_dir`.
    pub fn open(data: &'a [u8]) -> Result<Self, ZipError> {
        let mut archive_size = data.len() as u64;
        if archive_size < MZ_ZIP_END_OF_CENTRAL_DIR_HEADER_SIZE {
            return Err(ZipError::NotAnArchive);
        }
        let eocd_ofs = locate_eocd(data).ok_or(ZipError::FailedFindingCentralDir)?;
        let eocd = file_read(data, eocd_ofs, MZ_ZIP_END_OF_CENTRAL_DIR_HEADER_SIZE)
            .ok_or(ZipError::FileReadFailed)?;

        // zip64 detection: an EOCD64 locator directly before the EOCD.
        let mut zip64: Option<(&[u8], &[u8])> = None; // (locator, eocd64)
        if eocd_ofs
            >= MZ_ZIP64_END_OF_CENTRAL_DIR_LOCATOR_SIZE + MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIZE
        {
            let locator = file_read(
                data,
                eocd_ofs - MZ_ZIP64_END_OF_CENTRAL_DIR_LOCATOR_SIZE,
                MZ_ZIP64_END_OF_CENTRAL_DIR_LOCATOR_SIZE,
            )
            .ok_or(ZipError::FileReadFailed)?;
            if le32(locator, 0) == MZ_ZIP64_END_OF_CENTRAL_DIR_LOCATOR_SIG {
                // EOCD64 right before the locator, else where the locator says.
                let direct_ofs = eocd_ofs
                    - MZ_ZIP64_END_OF_CENTRAL_DIR_LOCATOR_SIZE
                    - MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIZE;
                let valid = |b: &&[u8]| le32(b, 0) == MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIG;
                let eocd64 =
                    match file_read(data, direct_ofs, MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIZE)
                        .filter(valid)
                    {
                        Some(b) => b,
                        None => {
                            let rel_ofs = le64(locator, 8);
                            if rel_ofs > archive_size - MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIZE {
                                return Err(ZipError::NotAnArchive);
                            }
                            file_read(data, rel_ofs, MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIZE)
                                .filter(valid)
                                .ok_or(ZipError::NotAnArchive)?
                        }
                    };
                zip64 = Some((locator, eocd64));
            }
        }

        let mut total_files = u32::from(le16(eocd, 10));
        let mut cdir_entries_on_this_disk = u32::from(le16(eocd, 8));
        let mut num_this_disk = u32::from(le16(eocd, 4));
        let mut cdir_disk_index = u32::from(le16(eocd, 6));
        let mut cdir_size = le32(eocd, 12);
        let mut cdir_ofs = u64::from(le32(eocd, 16));

        if let Some((locator, eocd64)) = zip64 {
            if le64(eocd64, 4) < MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIZE - 12 {
                return Err(ZipError::InvalidHeaderOrCorrupted);
            }
            if le32(locator, 16) != 1 {
                return Err(ZipError::UnsupportedMultidisk);
            }
            let entries = le64(eocd64, 32);
            if entries > u64::from(u32::MAX) {
                return Err(ZipError::TooManyFiles);
            }
            total_files = entries as u32;
            let entries_on_disk = le64(eocd64, 24);
            if entries_on_disk > u64::from(u32::MAX) {
                return Err(ZipError::TooManyFiles);
            }
            cdir_entries_on_this_disk = entries_on_disk as u32;
            let size_of_central_directory = le64(eocd64, 40);
            if size_of_central_directory > u64::from(u32::MAX) {
                return Err(ZipError::UnsupportedCdirSize);
            }
            cdir_size = size_of_central_directory as u32;
            num_this_disk = le32(eocd64, 16);
            cdir_disk_index = le32(eocd64, 20);
            cdir_ofs = le64(eocd64, 48);
        }

        if total_files != cdir_entries_on_this_disk {
            return Err(ZipError::UnsupportedMultidisk);
        }
        if (num_this_disk | cdir_disk_index) != 0 && (num_this_disk != 1 || cdir_disk_index != 1) {
            return Err(ZipError::UnsupportedMultidisk);
        }
        if u64::from(cdir_size) < u64::from(total_files) * u64::from(MZ_ZIP_CENTRAL_DIR_HEADER_SIZE)
        {
            return Err(ZipError::InvalidHeaderOrCorrupted);
        }
        let cdir_end = cdir_ofs.wrapping_add(u64::from(cdir_size));
        if cdir_end > archive_size {
            return Err(ZipError::InvalidHeaderOrCorrupted);
        }
        if eocd_ofs < cdir_end {
            return Err(ZipError::InvalidHeaderOrCorrupted);
        }
        // Anything between the central dir and the EOCD is a presumed archive
        // prefix; m_archive_size shrinks but reads stay absolute (TYPE_USER).
        let mut archive_ofs = eocd_ofs - cdir_end;
        if zip64.is_some() {
            if archive_ofs
                < MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIZE + MZ_ZIP64_END_OF_CENTRAL_DIR_LOCATOR_SIZE
            {
                return Err(ZipError::InvalidHeaderOrCorrupted);
            }
            archive_ofs -=
                MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIZE + MZ_ZIP64_END_OF_CENTRAL_DIR_LOCATOR_SIZE;
        }
        archive_size -= archive_ofs;

        let cdir: &[u8] = if total_files != 0 {
            file_read(data, cdir_ofs, u64::from(cdir_size)).ok_or(ZipError::FileReadFailed)?
        } else {
            &[]
        };

        let mut entry_ofs = Vec::new();
        entry_ofs
            .try_reserve_exact(total_files as usize)
            .map_err(|_| ZipError::AllocFailed)?;
        let mut zip64_has_extended_info_fields = false;
        let mut p = 0usize;
        let mut n = cdir_size;
        for _ in 0..total_files {
            if n < MZ_ZIP_CENTRAL_DIR_HEADER_SIZE || le32(cdir, p) != MZ_ZIP_CENTRAL_DIR_HEADER_SIG
            {
                return Err(ZipError::InvalidHeaderOrCorrupted);
            }
            entry_ofs.push(p as u32);
            let comp_size = le32(cdir, p + 20);
            let decomp_size = le32(cdir, p + 24);
            let local_header_ofs = le32(cdir, p + 42);
            let filename_size = u32::from(le16(cdir, p + 28));
            let ext_data_size = u32::from(le16(cdir, p + 30));

            if !zip64_has_extended_info_fields
                && ext_data_size != 0
                && comp_size.max(decomp_size).max(local_header_ofs) == u32::MAX
            {
                // Probe the extra data for a zip64 extended information field.
                let extra: &[u8] =
                    if MZ_ZIP_CENTRAL_DIR_HEADER_SIZE + filename_size + ext_data_size > n {
                        // miniz re-reads relative to the central dir START,
                        // not this entry (bug, kept for parity).
                        file_read(
                            data,
                            cdir_ofs
                                + u64::from(MZ_ZIP_CENTRAL_DIR_HEADER_SIZE)
                                + u64::from(filename_size),
                            u64::from(ext_data_size),
                        )
                        .ok_or(ZipError::FileReadFailed)?
                    } else {
                        &cdir[p + 46 + filename_size as usize..][..ext_data_size as usize]
                    };
                let mut q = 0usize;
                let mut extra_size_remaining = ext_data_size;
                loop {
                    if extra_size_remaining < 4 {
                        return Err(ZipError::InvalidHeaderOrCorrupted);
                    }
                    let field_id = le16(extra, q);
                    let field_data_size = u32::from(le16(extra, q + 2));
                    if field_data_size + 4 > extra_size_remaining {
                        return Err(ZipError::InvalidHeaderOrCorrupted);
                    }
                    if field_id == MZ_ZIP64_EXTENDED_INFORMATION_FIELD_HEADER_ID {
                        zip64_has_extended_info_fields = true;
                        break;
                    }
                    q += 4 + field_data_size as usize;
                    extra_size_remaining -= 4 + field_data_size;
                    if extra_size_remaining == 0 {
                        break;
                    }
                }
            }

            if comp_size != u32::MAX && decomp_size != u32::MAX {
                // The C reads a 32-bit word at the method offset, so this only
                // fires when method AND dos time are both zero.
                let method_and_time = le32(cdir, p + 10);
                if (method_and_time == 0 && decomp_size != comp_size)
                    || (decomp_size != 0 && comp_size == 0)
                {
                    return Err(ZipError::InvalidHeaderOrCorrupted);
                }
            }

            let disk_index = u32::from(le16(cdir, p + 34));
            if disk_index == u32::from(u16::MAX) || (disk_index != num_this_disk && disk_index != 1)
            {
                return Err(ZipError::UnsupportedMultidisk);
            }

            if comp_size != u32::MAX
                && u64::from(local_header_ofs) + MZ_ZIP_LOCAL_DIR_HEADER_SIZE + u64::from(comp_size)
                    > archive_size
            {
                return Err(ZipError::InvalidHeaderOrCorrupted);
            }

            let bit_flags = le16(cdir, p + 8);
            if bit_flags & MZ_ZIP_GENERAL_PURPOSE_BIT_FLAG_LOCAL_DIR_IS_MASKED != 0 {
                return Err(ZipError::UnsupportedEncryption);
            }

            let total_header_size = MZ_ZIP_CENTRAL_DIR_HEADER_SIZE
                + filename_size
                + ext_data_size
                + u32::from(le16(cdir, p + 32));
            if total_header_size > n {
                return Err(ZipError::InvalidHeaderOrCorrupted);
            }
            n -= total_header_size;
            p += total_header_size as usize;
        }

        let mut sorted: Vec<u32> = (0..total_files).collect();
        sort_central_dir_offsets_by_filename(cdir, &entry_ofs, &mut sorted);

        Ok(ZipArchive {
            data,
            archive_size,
            cdir,
            entry_ofs,
            sorted,
        })
    }

    /// `mz_zip_reader_extract_file_to_heap(&archive, name, &size, 0)`.
    pub fn extract(&self, name: &[u8]) -> Result<Vec<u8>, ZipError> {
        let file_index = self.locate_file(name)?;
        self.extract_to_heap(file_index)
    }

    /// `mz_zip_reader_locate_file_v2` with flags 0 and no comment: binary
    /// search over the case-insensitively sorted central dir.
    fn locate_file(&self, name: &[u8]) -> Result<u32, ZipError> {
        if self.sorted.is_empty() {
            // The C falls through to the linear scan, whose only observable
            // effects on an empty archive are these two errors.
            if name.len() > usize::from(u16::MAX) {
                return Err(ZipError::InvalidParameter);
            }
            return Err(ZipError::FileNotFound);
        }
        let mut l: i64 = 0;
        let mut h: i64 = self.sorted.len() as i64 - 1;
        while l <= h {
            let m = l + ((h - l) >> 1);
            let file_index = self.sorted[usize::try_from(m).unwrap_or_default()];
            let comp = filename_compare(self.cdir, self.entry_ofs[file_index as usize], name);
            if comp == 0 {
                return Ok(file_index);
            }
            if comp < 0 {
                l = m + 1;
            } else {
                h = m - 1;
            }
        }
        Err(ZipError::FileNotFound)
    }

    /// `mz_zip_file_stat_internal` (extraction-relevant fields).
    fn file_stat(&self, file_index: u32) -> Result<FileStat, ZipError> {
        let cdir = self.cdir;
        let p = self.entry_ofs[file_index as usize] as usize;
        let bit_flag = le16(cdir, p + 8);
        let method = le16(cdir, p + 10);
        let crc32 = le32(cdir, p + 16);
        let mut comp_size = u64::from(le32(cdir, p + 20));
        let mut uncomp_size = u64::from(le32(cdir, p + 24));
        let mut local_header_ofs = u64::from(le32(cdir, p + 42));
        let filename_len = le16(cdir, p + 28) as usize;
        // mz_zip_reader_is_file_a_directory
        let is_directory = (filename_len != 0 && cdir[p + 46 + filename_len - 1] == b'/')
            || le32(cdir, p + 38) & MZ_ZIP_DOS_DIR_ATTRIBUTE_BITFLAG != 0;

        if comp_size.max(uncomp_size).max(local_header_ofs) == u64::from(u32::MAX) {
            let ext_len = le16(cdir, p + 30) as usize;
            if ext_len != 0 {
                let extra = &cdir[p + 46 + filename_len..][..ext_len];
                let mut q = 0usize;
                let mut extra_size_remaining = ext_len;
                loop {
                    if extra_size_remaining < 4 {
                        return Err(ZipError::InvalidHeaderOrCorrupted);
                    }
                    let field_id = le16(extra, q);
                    let field_data_size = le16(extra, q + 2) as usize;
                    if field_data_size + 4 > extra_size_remaining {
                        return Err(ZipError::InvalidHeaderOrCorrupted);
                    }
                    if field_id == MZ_ZIP64_EXTENDED_INFORMATION_FIELD_HEADER_ID {
                        let mut f = q + 4;
                        let mut field_data_remaining = field_data_size;
                        if uncomp_size == u64::from(u32::MAX) {
                            if field_data_remaining < 8 {
                                return Err(ZipError::InvalidHeaderOrCorrupted);
                            }
                            uncomp_size = le64(extra, f);
                            f += 8;
                            field_data_remaining -= 8;
                        }
                        if comp_size == u64::from(u32::MAX) {
                            if field_data_remaining < 8 {
                                return Err(ZipError::InvalidHeaderOrCorrupted);
                            }
                            comp_size = le64(extra, f);
                            f += 8;
                            field_data_remaining -= 8;
                        }
                        if local_header_ofs == u64::from(u32::MAX) {
                            if field_data_remaining < 8 {
                                return Err(ZipError::InvalidHeaderOrCorrupted);
                            }
                            local_header_ofs = le64(extra, f);
                        }
                        break;
                    }
                    q += 4 + field_data_size;
                    extra_size_remaining -= 4 + field_data_size;
                    if extra_size_remaining == 0 {
                        break;
                    }
                }
            }
        }

        Ok(FileStat {
            bit_flag,
            method,
            crc32,
            comp_size,
            uncomp_size,
            local_header_ofs,
            is_directory,
        })
    }

    /// `mz_zip_reader_extract_to_heap` with flags 0: allocate `uncomp_size`,
    /// then `mz_zip_reader_extract_to_mem_no_alloc1`.
    fn extract_to_heap(&self, file_index: u32) -> Result<Vec<u8>, ZipError> {
        let file_stat = self.file_stat(file_index)?;
        let alloc_size =
            usize::try_from(file_stat.uncomp_size).map_err(|_| ZipError::AllocFailed)?;
        let mut buf = Vec::new();
        buf.try_reserve_exact(alloc_size)
            .map_err(|_| ZipError::AllocFailed)?;
        buf.resize(alloc_size, 0);
        self.extract_to_mem(&file_stat, &mut buf)?;
        Ok(buf)
    }

    /// `mz_zip_reader_extract_to_mem_no_alloc1` with flags 0.
    fn extract_to_mem(&self, file_stat: &FileStat, buf: &mut [u8]) -> Result<(), ZipError> {
        // A directory or zero length file: success, output untouched.
        if file_stat.is_directory || file_stat.comp_size == 0 {
            return Ok(());
        }
        if file_stat.bit_flag
            & (MZ_ZIP_GENERAL_PURPOSE_BIT_FLAG_IS_ENCRYPTED
                | MZ_ZIP_GENERAL_PURPOSE_BIT_FLAG_USES_STRONG_ENCRYPTION
                | MZ_ZIP_GENERAL_PURPOSE_BIT_FLAG_COMPRESSED_PATCH_FLAG)
            != 0
        {
            return Err(ZipError::UnsupportedEncryption);
        }
        if file_stat.method != 0 && file_stat.method != MZ_DEFLATED {
            return Err(ZipError::UnsupportedMethod);
        }

        let local_header = file_read(
            self.data,
            file_stat.local_header_ofs,
            MZ_ZIP_LOCAL_DIR_HEADER_SIZE,
        )
        .ok_or(ZipError::FileReadFailed)?;
        if le32(local_header, 0) != MZ_ZIP_LOCAL_DIR_HEADER_SIG {
            return Err(ZipError::InvalidHeaderOrCorrupted);
        }
        let cur_file_ofs = file_stat.local_header_ofs.wrapping_add(
            MZ_ZIP_LOCAL_DIR_HEADER_SIZE
                + u64::from(le16(local_header, 26))
                + u64::from(le16(local_header, 28)),
        );
        if cur_file_ofs.wrapping_add(file_stat.comp_size) > self.archive_size {
            return Err(ZipError::InvalidHeaderOrCorrupted);
        }

        if file_stat.method == 0 {
            // Stored: the C reads uncomp_size bytes, not comp_size.
            let src = file_read(self.data, cur_file_ofs, file_stat.uncomp_size)
                .ok_or(ZipError::FileReadFailed)?;
            buf.copy_from_slice(src);
            if mz_crc32(buf) != file_stat.crc32 {
                return Err(ZipError::CrcCheckFailed);
            }
            return Ok(());
        }

        // Deflate. The C reads 64KB chunks; when comp_size overruns the file
        // (reachable only via the u64 wraparound above) only whole chunks are
        // delivered before the failing read aborts the loop.
        let avail = (self.data.len() as u64).saturating_sub(cur_file_ofs);
        let (input, truncated_feed) = if file_stat.comp_size <= avail {
            let src = file_read(self.data, cur_file_ofs, file_stat.comp_size)
                .ok_or(ZipError::DecompressionFailed)?;
            (src, false)
        } else {
            let delivered = avail / MZ_ZIP_MAX_IO_BUF_SIZE * MZ_ZIP_MAX_IO_BUF_SIZE;
            let src = file_read(self.data, cur_file_ofs, delivered)
                .ok_or(ZipError::DecompressionFailed)?;
            (src, true)
        };
        let mut flags = TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
        if truncated_feed {
            flags |= TINFL_FLAG_HAS_MORE_INPUT;
        }
        let mut inflator = DecompressorOxide::new();
        let (status, _in_consumed, out_written) = decompress(&mut inflator, input, buf, 0, flags);
        if status != TINFLStatus::Done {
            // Covers tinfl failures and, in the truncated-feed case, the
            // failing chunk read (NeedsMoreInput -> MZ_ZIP_DECOMPRESSION_FAILED).
            return Err(ZipError::DecompressionFailed);
        }
        if out_written as u64 != file_stat.uncomp_size {
            return Err(ZipError::UnexpectedDecompressedSize);
        }
        if mz_crc32(buf) != file_stat.crc32 {
            return Err(ZipError::CrcCheckFailed);
        }
        Ok(())
    }
}

/// The embedded vkquake.pak decompression in `COM_AddGameDirectoryRoot`:
/// raw `tinfl_decompress` with `TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF`
/// into a buffer of `decompressed_size`; the only success criterion is
/// `TINFL_STATUS_DONE`, and the C keeps whatever length was produced.
pub fn inflate_embedded(compressed: &[u8], decompressed_size: usize) -> Result<Vec<u8>, ZipError> {
    let mut out = vec![0u8; decompressed_size];
    let mut inflator = DecompressorOxide::new();
    let (status, _in_consumed, out_written) = decompress(
        &mut inflator,
        compressed,
        &mut out,
        0,
        TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
    );
    if status != TINFLStatus::Done {
        return Err(ZipError::DecompressionFailed);
    }
    out.truncate(out_written);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ZEntry {
        name: Vec<u8>,
        payload: Vec<u8>,
        method: u16,
        crc: u32,
        uncomp_size: u32,
        bit_flag: u16,
        time: u16,
        ext_attr: u32,
    }

    impl ZEntry {
        fn stored(name: &str, data: &[u8]) -> Self {
            ZEntry {
                name: name.as_bytes().to_vec(),
                payload: data.to_vec(),
                method: 0,
                crc: mz_crc32(data),
                uncomp_size: data.len() as u32,
                bit_flag: 0,
                time: 0,
                ext_attr: 0,
            }
        }

        fn deflated(name: &str, data: &[u8]) -> Self {
            ZEntry {
                name: name.as_bytes().to_vec(),
                payload: miniz_oxide::deflate::compress_to_vec(data, 6),
                method: MZ_DEFLATED,
                crc: mz_crc32(data),
                uncomp_size: data.len() as u32,
                bit_flag: 0,
                time: 0,
                ext_attr: 0,
            }
        }
    }

    fn build_zip(entries: &[ZEntry], comment: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut local_ofs = Vec::new();
        for e in entries {
            local_ofs.push(out.len() as u32);
            out.extend_from_slice(&MZ_ZIP_LOCAL_DIR_HEADER_SIG.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&e.bit_flag.to_le_bytes());
            out.extend_from_slice(&e.method.to_le_bytes());
            out.extend_from_slice(&e.time.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // date
            out.extend_from_slice(&e.crc.to_le_bytes());
            out.extend_from_slice(&(e.payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&e.uncomp_size.to_le_bytes());
            out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(&e.name);
            out.extend_from_slice(&e.payload);
        }
        let cdir_ofs = out.len() as u32;
        for (e, &lofs) in entries.iter().zip(&local_ofs) {
            out.extend_from_slice(&MZ_ZIP_CENTRAL_DIR_HEADER_SIG.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes()); // version made by
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&e.bit_flag.to_le_bytes());
            out.extend_from_slice(&e.method.to_le_bytes());
            out.extend_from_slice(&e.time.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // date
            out.extend_from_slice(&e.crc.to_le_bytes());
            out.extend_from_slice(&(e.payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&e.uncomp_size.to_le_bytes());
            out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra len
            out.extend_from_slice(&0u16.to_le_bytes()); // comment len
            out.extend_from_slice(&0u16.to_le_bytes()); // disk start
            out.extend_from_slice(&0u16.to_le_bytes()); // internal attr
            out.extend_from_slice(&e.ext_attr.to_le_bytes());
            out.extend_from_slice(&lofs.to_le_bytes());
            out.extend_from_slice(&e.name);
        }
        let cdir_size = out.len() as u32 - cdir_ofs;
        out.extend_from_slice(&MZ_ZIP_END_OF_CENTRAL_DIR_HEADER_SIG.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // this disk
        out.extend_from_slice(&0u16.to_le_bytes()); // cdir disk
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&cdir_size.to_le_bytes());
        out.extend_from_slice(&cdir_ofs.to_le_bytes());
        out.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        out.extend_from_slice(comment);
        out
    }

    #[test]
    fn stored_round_trip() {
        let zip = build_zip(&[ZEntry::stored("dir/a.txt", b"hello stored")], b"");
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(archive.extract(b"dir/a.txt").unwrap(), b"hello stored");
        assert_eq!(archive.extract(b"missing"), Err(ZipError::FileNotFound));
    }

    #[test]
    fn deflate_round_trip() {
        let data: Vec<u8> = (0..2000u32).flat_map(|i| (i % 251).to_le_bytes()).collect();
        let zip = build_zip(
            &[
                ZEntry::deflated("localization/loc_english.txt", &data),
                ZEntry::deflated("empty.bin", b""),
            ],
            b"",
        );
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(
            archive.extract(b"localization/loc_english.txt").unwrap(),
            data
        );
        assert_eq!(archive.extract(b"empty.bin").unwrap(), b"");
    }

    #[test]
    fn name_lookup_case_insensitive_full_path() {
        let zip = build_zip(&[ZEntry::stored("Dir/File.TXT", b"x")], b"");
        let archive = ZipArchive::open(&zip).unwrap();
        // flags=0: case-insensitive...
        assert_eq!(archive.extract(b"dir/file.txt").unwrap(), b"x");
        assert_eq!(archive.extract(b"DIR/FILE.TXT").unwrap(), b"x");
        // ...but the path is NOT ignored (no MZ_ZIP_FLAG_IGNORE_PATH).
        assert_eq!(archive.extract(b"File.TXT"), Err(ZipError::FileNotFound));
    }

    #[test]
    fn multi_entry_sorted_lookup() {
        // Deliberately unsorted so the heap sort + binary search do real work.
        let zip = build_zip(
            &[
                ZEntry::stored("zeta.txt", b"zzz"),
                ZEntry::stored("alpha/beta.txt", b"ab"),
                ZEntry::deflated("Mango.bin", b"mango mango mango"),
                ZEntry::stored("alpha.txt", b"a"),
            ],
            b"",
        );
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(archive.extract(b"zeta.txt").unwrap(), b"zzz");
        assert_eq!(archive.extract(b"alpha/beta.txt").unwrap(), b"ab");
        assert_eq!(archive.extract(b"mango.BIN").unwrap(), b"mango mango mango");
        assert_eq!(archive.extract(b"alpha.txt").unwrap(), b"a");
    }

    #[test]
    fn eocd_comment_scan_back() {
        let zip = build_zip(
            &[ZEntry::stored("a", b"data")],
            b"vkQuake kpf archive comment",
        );
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(archive.extract(b"a").unwrap(), b"data");
    }

    #[test]
    fn eocd_scan_distance_limit() {
        // Trailing junk within the ~64KB scan window: still found.
        let mut ok = build_zip(&[], b"");
        ok.extend_from_slice(&vec![0u8; 60_000]);
        assert!(ZipArchive::open(&ok).is_ok());
        // Beyond the chunked give-up threshold: not found.
        let mut lost = build_zip(&[], b"");
        lost.extend_from_slice(&vec![0u8; 70_000]);
        assert_eq!(
            ZipArchive::open(&lost).err(),
            Some(ZipError::FailedFindingCentralDir)
        );
    }

    #[test]
    fn missing_or_bogus_eocd() {
        assert_eq!(
            ZipArchive::open(&[0u8; 10]).err(),
            Some(ZipError::NotAnArchive)
        );
        assert_eq!(
            ZipArchive::open(&[0u8; 100]).err(),
            Some(ZipError::FailedFindingCentralDir)
        );
        // A sig with less than 22 bytes of room does not count.
        let mut d = vec![0u8; 30];
        d[26..30].copy_from_slice(&MZ_ZIP_END_OF_CENTRAL_DIR_HEADER_SIG.to_le_bytes());
        assert_eq!(
            ZipArchive::open(&d).err(),
            Some(ZipError::FailedFindingCentralDir)
        );
    }

    #[test]
    fn corrupt_central_dir() {
        let good = build_zip(&[ZEntry::stored("a.txt", b"payload")], b"");
        // cdir_ofs pushed past the archive: cdir_ofs + cdir_size > size.
        let mut bad = good.clone();
        let eocd = bad.len() - 22;
        let ofs = le32(&bad, eocd + 16) + 1000;
        bad[eocd + 16..eocd + 20].copy_from_slice(&ofs.to_le_bytes());
        assert_eq!(
            ZipArchive::open(&bad).err(),
            Some(ZipError::InvalidHeaderOrCorrupted)
        );
        // Corrupt central dir header signature.
        let mut bad = good.clone();
        let cdir_ofs = le32(&good, eocd + 16) as usize;
        bad[cdir_ofs] ^= 0xff;
        assert_eq!(
            ZipArchive::open(&bad).err(),
            Some(ZipError::InvalidHeaderOrCorrupted)
        );
    }

    #[test]
    fn bad_local_header_magic() {
        let mut zip = build_zip(&[ZEntry::stored("a.txt", b"payload")], b"");
        zip[0] ^= 0xff; // local header sig; open never looks at it
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(
            archive.extract(b"a.txt"),
            Err(ZipError::InvalidHeaderOrCorrupted)
        );
    }

    #[test]
    fn crc_mismatch() {
        let mut e = ZEntry::stored("a.txt", b"payload");
        e.crc ^= 1;
        let zip = build_zip(&[e], b"");
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(archive.extract(b"a.txt"), Err(ZipError::CrcCheckFailed));

        let data = b"deflate me deflate me deflate me";
        let mut e = ZEntry::deflated("b.txt", data);
        e.crc ^= 1;
        let zip = build_zip(&[e], b"");
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(archive.extract(b"b.txt"), Err(ZipError::CrcCheckFailed));
    }

    #[test]
    fn empty_archive() {
        let zip = build_zip(&[], b"");
        assert_eq!(zip.len(), 22);
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(archive.extract(b"anything"), Err(ZipError::FileNotFound));
    }

    #[test]
    fn encryption_flags() {
        // Encrypted entry: accepted at open, rejected at extract.
        let mut e = ZEntry::stored("a", b"x");
        e.bit_flag = MZ_ZIP_GENERAL_PURPOSE_BIT_FLAG_IS_ENCRYPTED;
        let zip = build_zip(&[e], b"");
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(archive.extract(b"a"), Err(ZipError::UnsupportedEncryption));
        // Masked local dir: rejected at open.
        let mut e = ZEntry::stored("a", b"x");
        e.bit_flag = MZ_ZIP_GENERAL_PURPOSE_BIT_FLAG_LOCAL_DIR_IS_MASKED;
        let zip = build_zip(&[e], b"");
        assert_eq!(
            ZipArchive::open(&zip).err(),
            Some(ZipError::UnsupportedEncryption)
        );
    }

    #[test]
    fn unsupported_method() {
        let mut e = ZEntry::stored("a", b"x");
        e.method = 99;
        let zip = build_zip(&[e], b"");
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(archive.extract(b"a"), Err(ZipError::UnsupportedMethod));
    }

    #[test]
    fn stored_size_mismatch_method_and_time_quirk() {
        // method == 0 and time == 0: comp != uncomp is rejected at open.
        let mut e = ZEntry::stored("a", b"hello");
        e.uncomp_size = 3;
        e.crc = mz_crc32(b"hel");
        let zip = build_zip(&[e], b"");
        assert_eq!(
            ZipArchive::open(&zip).err(),
            Some(ZipError::InvalidHeaderOrCorrupted)
        );
        // Nonzero DOS time defeats the 32-bit read at the method offset, the
        // mismatch is tolerated, and extraction reads uncomp_size bytes.
        let mut e = ZEntry::stored("a", b"hello");
        e.uncomp_size = 3;
        e.crc = mz_crc32(b"hel");
        e.time = 1;
        let zip = build_zip(&[e], b"");
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(archive.extract(b"a").unwrap(), b"hel");
    }

    #[test]
    fn directory_entries_extract_untouched() {
        // Trailing '/' name and DOS dir attribute both short-circuit to
        // success; miniz hands back uninitialized memory, we hand back zeros.
        let mut by_attr = ZEntry::stored("weird.bin", b"junk");
        by_attr.ext_attr = MZ_ZIP_DOS_DIR_ATTRIBUTE_BITFLAG;
        by_attr.crc = 0xdead_beef; // never checked
        let zip = build_zip(&[ZEntry::stored("sub/dir/", b""), by_attr], b"");
        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(archive.extract(b"sub/dir/").unwrap(), b"");
        assert_eq!(archive.extract(b"weird.bin").unwrap(), vec![0u8; 4]);
    }

    #[test]
    fn zip64_records() {
        let data = b"zip64 payload data";
        let payload_crc = mz_crc32(data);
        let name = b"z64.bin";
        let mut zip = Vec::new();
        // local header (real sizes)
        zip.extend_from_slice(&MZ_ZIP_LOCAL_DIR_HEADER_SIG.to_le_bytes());
        zip.extend_from_slice(&45u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes()); // stored
        zip.extend_from_slice(&0u32.to_le_bytes()); // time+date
        zip.extend_from_slice(&payload_crc.to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(name);
        zip.extend_from_slice(data);
        // central dir entry: sizes/offset deferred to the zip64 extra field
        let cdir_ofs = zip.len() as u64;
        zip.extend_from_slice(&MZ_ZIP_CENTRAL_DIR_HEADER_SIG.to_le_bytes());
        zip.extend_from_slice(&45u16.to_le_bytes());
        zip.extend_from_slice(&45u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes()); // stored
        zip.extend_from_slice(&0u32.to_le_bytes()); // time+date
        zip.extend_from_slice(&payload_crc.to_le_bytes());
        zip.extend_from_slice(&u32::MAX.to_le_bytes()); // comp
        zip.extend_from_slice(&u32::MAX.to_le_bytes()); // uncomp
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&28u16.to_le_bytes()); // extra len
        zip.extend_from_slice(&0u16.to_le_bytes()); // comment
        zip.extend_from_slice(&0u16.to_le_bytes()); // disk
        zip.extend_from_slice(&0u16.to_le_bytes()); // int attr
        zip.extend_from_slice(&0u32.to_le_bytes()); // ext attr
        zip.extend_from_slice(&u32::MAX.to_le_bytes()); // local ofs
        zip.extend_from_slice(name);
        zip.extend_from_slice(&MZ_ZIP64_EXTENDED_INFORMATION_FIELD_HEADER_ID.to_le_bytes());
        zip.extend_from_slice(&24u16.to_le_bytes());
        zip.extend_from_slice(&(data.len() as u64).to_le_bytes()); // uncomp
        zip.extend_from_slice(&(data.len() as u64).to_le_bytes()); // comp
        zip.extend_from_slice(&0u64.to_le_bytes()); // local header ofs
        let cdir_size = zip.len() as u64 - cdir_ofs;
        // EOCD64
        let eocd64_ofs = zip.len() as u64;
        zip.extend_from_slice(&MZ_ZIP64_END_OF_CENTRAL_DIR_HEADER_SIG.to_le_bytes());
        zip.extend_from_slice(&44u64.to_le_bytes()); // size of record
        zip.extend_from_slice(&45u16.to_le_bytes());
        zip.extend_from_slice(&45u16.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes()); // this disk
        zip.extend_from_slice(&0u32.to_le_bytes()); // cdir disk
        zip.extend_from_slice(&1u64.to_le_bytes()); // entries on disk
        zip.extend_from_slice(&1u64.to_le_bytes()); // total entries
        zip.extend_from_slice(&cdir_size.to_le_bytes());
        zip.extend_from_slice(&cdir_ofs.to_le_bytes());
        // EOCD64 locator
        zip.extend_from_slice(&MZ_ZIP64_END_OF_CENTRAL_DIR_LOCATOR_SIG.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&eocd64_ofs.to_le_bytes());
        zip.extend_from_slice(&1u32.to_le_bytes()); // total disks
                                                    // EOCD
        zip.extend_from_slice(&MZ_ZIP_END_OF_CENTRAL_DIR_HEADER_SIG.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&(cdir_size as u32).to_le_bytes());
        zip.extend_from_slice(&(cdir_ofs as u32).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());

        let archive = ZipArchive::open(&zip).unwrap();
        assert_eq!(archive.extract(b"z64.bin").unwrap(), data);
    }

    #[test]
    fn inflate_embedded_round_trip() {
        let data: Vec<u8> = (0..5000u32).flat_map(|i| (i % 199).to_le_bytes()).collect();
        let compressed = miniz_oxide::deflate::compress_to_vec(&data, 10);
        // Exact-size buffer.
        assert_eq!(inflate_embedded(&compressed, data.len()).unwrap(), data);
        // Oversized buffer: the C only checks TINFL_STATUS_DONE and keeps the
        // produced length.
        assert_eq!(
            inflate_embedded(&compressed, data.len() + 100).unwrap(),
            data
        );
    }

    #[test]
    fn inflate_embedded_failures() {
        let data = vec![7u8; 1000];
        let compressed = miniz_oxide::deflate::compress_to_vec(&data, 6);
        // Output buffer too small: not DONE.
        assert_eq!(
            inflate_embedded(&compressed, 10),
            Err(ZipError::DecompressionFailed)
        );
        // Truncated stream: not DONE.
        assert_eq!(
            inflate_embedded(&compressed[..compressed.len() / 2], 1000),
            Err(ZipError::DecompressionFailed)
        );
        // Garbage input.
        assert_eq!(
            inflate_embedded(&[0xff; 16], 1000),
            Err(ZipError::DecompressionFailed)
        );
    }

    #[test]
    fn crc32_matches_known_vectors() {
        assert_eq!(mz_crc32(b""), 0);
        assert_eq!(mz_crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(mz_crc32(b"hello"), 0x3610_a686);
    }
}
