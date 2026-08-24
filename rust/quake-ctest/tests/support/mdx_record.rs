//! Readers for the MD3/MD5 call recorders in `stubs/stubs.c` (Phase 3 M5).
//!
//! `GLMesh_UploadBuffers` is where the *parsed* MD3/MD5 payload goes — the
//! index, vertex, texcoord and joint buffers never land in `aliashdr_t` —
//! so the stub copies those buffers and this module hands them to the
//! differential as one comparable value. The recorded byte lengths are
//! derived by the stub from the header the loader just filled, identically
//! for both sides; the tests additionally assert them against what each
//! fixture implies, so a mis-derived length fails loudly instead of quietly
//! comparing nothing.

#![allow(dead_code)]

use core::ffi::c_char;

const CTEST_UPLOAD_BYTES: usize = 65536;
const CTEST_UPLOAD_MAX: i32 = 16;

/// Mirror of `ctest_upload_call_t`.
#[repr(C)]
pub struct UploadCall {
    numverts: i32,
    numverts_vbo: i32,
    numtris: i32,
    numindexes: i32,
    numposes: i32,
    numframes: i32,
    numjoints: i32,
    poseverttype: i32,
    numskins: i32,
    has_next_surface: i32,
    has_desc: i32,
    has_joints: i32,
    index_bytes: i32,
    vertex_bytes: i32,
    desc_bytes: i32,
    joint_bytes: i32,
    index_hash: u64,
    vertex_hash: u64,
    desc_hash: u64,
    joint_hash: u64,
    truncated: i32,
    data: [u8; CTEST_UPLOAD_BYTES],
}

/// Mirror of `ctest_mdxskin_call_t`.
#[repr(C)]
pub struct MdxSkinCall {
    name: [c_char; 64],
    surf_index: i32,
    numsurfaces: i64,
    numskins: i64,
    kind: i32,
}

extern "C" {
    pub fn ctest_mdxstub_reset(skins_result: i32);
    fn ctest_upload_count() -> i32;
    fn ctest_upload_calls() -> *const UploadCall;
    fn ctest_mdxskin_count() -> i32;
    fn ctest_mdxskin_calls() -> *const MdxSkinCall;
    pub fn ctest_skindefs_count() -> i32;
    pub fn ctest_deletemesh_count() -> i32;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Upload {
    pub numverts: i32,
    pub numverts_vbo: i32,
    pub numtris: i32,
    pub numindexes: i32,
    pub numposes: i32,
    pub numframes: i32,
    pub numjoints: i32,
    pub poseverttype: i32,
    pub numskins: i32,
    pub has_next_surface: bool,
    pub has_desc: bool,
    pub has_joints: bool,
    pub index_bytes: i32,
    pub vertex_bytes: i32,
    pub desc_bytes: i32,
    pub joint_bytes: i32,
    /// FNV-1a of each buffer; always meaningful, even when the byte copy
    /// below was skipped because the buffer outgrew the recorder
    pub hashes: [u64; 4],
    /// empty when [`Upload::truncated`] is set
    pub payload: Vec<u8>,
    pub truncated: bool,
}

impl Upload {
    /// The vertex-buffer slice of the recorded payload.
    ///
    /// Panics on a truncated capture -- callers that may hit real,
    /// large models must check [`Upload::truncated`] and fall back to
    /// [`Upload::hashes`].
    pub fn vertex_payload(&self) -> &[u8] {
        assert!(!self.truncated, "payload was not captured (too large)");
        let a = self.index_bytes.max(0) as usize;
        &self.payload[a..a + self.vertex_bytes.max(0) as usize]
    }

    /// The joint-buffer slice of the recorded payload.
    pub fn joint_payload(&self) -> &[u8] {
        assert!(!self.truncated, "payload was not captured (too large)");
        let a = (self.index_bytes + self.vertex_bytes + self.desc_bytes).max(0) as usize;
        &self.payload[a..a + self.joint_bytes.max(0) as usize]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MdxSkin {
    pub name: String,
    pub surf_index: i32,
    pub numsurfaces: i64,
    pub numskins: i64,
    /// 0 = the MD5 callback, 1 = the MD3 one
    pub kind: i32,
}

fn cstr(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().map(|&c| c as u8).collect();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

impl From<&UploadCall> for Upload {
    fn from(c: &UploadCall) -> Self {
        let n = if c.truncated != 0 {
            0
        } else {
            (c.index_bytes + c.vertex_bytes + c.desc_bytes + c.joint_bytes).max(0) as usize
        };
        Upload {
            numverts: c.numverts,
            numverts_vbo: c.numverts_vbo,
            numtris: c.numtris,
            numindexes: c.numindexes,
            numposes: c.numposes,
            numframes: c.numframes,
            numjoints: c.numjoints,
            poseverttype: c.poseverttype,
            numskins: c.numskins,
            has_next_surface: c.has_next_surface != 0,
            has_desc: c.has_desc != 0,
            has_joints: c.has_joints != 0,
            index_bytes: c.index_bytes,
            vertex_bytes: c.vertex_bytes,
            desc_bytes: c.desc_bytes,
            joint_bytes: c.joint_bytes,
            hashes: [c.index_hash, c.vertex_hash, c.desc_hash, c.joint_hash],
            payload: c.data[..n].to_vec(),
            truncated: c.truncated != 0,
        }
    }
}

/// How many times `GLMesh_UploadBuffers` was called (may exceed the number
/// of *recorded* entries: the log is capped).
pub fn recorded_upload_count() -> i32 {
    // SAFETY: plain static counter
    unsafe { ctest_upload_count() }
}

/// Reads the upload recorder, clamped to the log's capacity. Caller must
/// hold the ctest fs lock.
pub fn recorded_uploads() -> Vec<Upload> {
    // SAFETY: the recorder holds min(count, CTEST_UPLOAD_MAX) valid entries
    unsafe {
        let n = ctest_upload_count().min(CTEST_UPLOAD_MAX);
        let p = ctest_upload_calls();
        (0..n as isize)
            .map(|i| Upload::from(&*p.offset(i)))
            .collect()
    }
}

/// Reads the skin-callback recorder. Caller must hold the ctest fs lock.
pub fn recorded_skins() -> Vec<MdxSkin> {
    // SAFETY: the recorder holds `ctest_mdxskin_count` valid entries
    unsafe {
        let n = ctest_mdxskin_count();
        let p = ctest_mdxskin_calls();
        (0..n as isize)
            .map(|i| {
                let c = &*p.offset(i);
                MdxSkin {
                    name: cstr(&c.name),
                    surf_index: c.surf_index,
                    numsurfaces: c.numsurfaces,
                    numskins: c.numskins,
                    kind: c.kind,
                }
            })
            .collect()
    }
}
