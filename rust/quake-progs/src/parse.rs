//! The savegame/entity-text value parser: `ED_NewString`, `ED_RezoneString`
//! and `ED_ParseEpair` (`pr_edict.c`).
//!
//! This is the read side of ADR-019's gate 2. `ED_ParseEdict` — the *key*
//! dispatcher — stays C for now: its `_precache_model`/`_precache_sound`
//! hacks, its `traileffect`/`emiteffect` PSET_SCRIPT branches and its
//! `sv.state` tests are server code Phase 7 owns. What moves here is the
//! value parser, which is where the compatibility risk actually lives.
//!
//! COMPAT (ADR-010): every numeric conversion goes through the platform's
//! `atof`/`atoi`/`strtoll`/`strtoull` rather than a Rust parser. `atof` is
//! `strtod`, whose rounding of a decimal savegame literal back to the binary
//! value is exactly the round-trip the byte-diff gate checks.

use core::ffi::{c_int, CStr};

use quake_types::progs::{etype, FreeList, DEF_SAVEGLOBAL};

use crate::alloc::{self, AllocError};
use crate::arena::{EdictArena, EdictId, Mem, VmRaw};

const _: () = assert!(
    cfg!(target_endian = "little"),
    "the 64-bit QC types are written low word first here; a big-endian target \
     needs the ordering derived from target_endian (see ED_ParseEpair)"
);

/// What the parser needs from code that has not moved yet.
pub trait ParseSys: Mem {
    /// The platform `atof` (i.e. `strtod`). Not reimplemented: its rounding is
    /// the savegame round-trip's contract (ADR-010).
    fn atof(&mut self, s: &CStr) -> f64;
    /// The platform `atoi`.
    fn atoi(&mut self, s: &CStr) -> c_int;
    /// The platform `strtoll (s, NULL, 0)` — note base 0, so `0x`/`0` prefixes
    /// are honoured.
    fn strtoll(&mut self, s: &CStr) -> i64;
    /// The platform `strtoull (s, NULL, 0)`.
    fn strtoull(&mut self, s: &CStr) -> u64;

    /// `ED_FindField` — returns the field's *global* offset, or `None`.
    fn find_field_ofs(&mut self, name: &CStr) -> Option<c_int>;
    /// `ED_FindFunction` — returns the function index, or `None`.
    fn find_function(&mut self, name: &CStr) -> Option<c_int>;
    /// `SV_UnlinkEdict`, reached through `ED_Free`.
    fn unlink_edict(&mut self, id: EdictId);

    /// `Con_DPrintf`, deferred: the console is not a leaf.
    ///
    /// The arguments are raw progs bytes, not `str`: Quake strings routinely
    /// carry high-bit bytes (the coloured-text charset), and lossy UTF-8
    /// conversion would print different bytes than C's `%s`.
    fn dprint(&mut self, prefix: &str, arg: &[u8], suffix: &str);
    /// `Con_Printf`, deferred.
    fn print(&mut self, prefix: &str, arg: &[u8], suffix: &str);
    /// `Con_DWarning`, deferred.
    fn dwarn(&mut self, prefix: &str, arg: &[u8], mid: &str, arg2: &[u8], suffix: &str);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// `ED_ParseEpair: ev_entity %d too large (max_edicts is %i)`
    EntityTooLarge { num: c_int, max_edicts: c_int },
    /// `ED_AddToFreeList : has more than max_edicts >= %i` — a `DEBUG`/
    /// `_DEBUG`-only check in C.
    FreeListFull {
        kind: alloc::FreeListOverflow,
        max_edicts: c_int,
    },
    /// `EDICT_NUM: bad edict_num %i`.
    ///
    /// C only range-checks the *upper* bound in `ED_ParseEpair` itself, but
    /// `EDICT_NUM` then rejects `n < 0` unconditionally (`pr_edict.c`), so a
    /// negative literal raises there instead. Reported here so the raise
    /// happens in the C frame (ADR-009).
    BadEdictNum(c_int),
    /// Propagated from `ED_Free`'s free-list bookkeeping.
    Alloc(AllocError),
}

/// `ED_NewString` — copies a string into a fresh engine string, translating
/// `\n` escapes.
///
/// COMPAT: the loop runs to `l = strlen + 1` inclusive of the NUL, and an
/// escape at the very end (`i == l - 1`) is *not* treated as an escape, so a
/// trailing backslash survives verbatim. A `\` followed by anything other than
/// `n` becomes a lone `\`, dropping the second character — which shortens the
/// string, leaving the tail of the allocation as the zeroes `Mem_Alloc` gave.
pub fn ed_new_string(vm: &mut VmRaw, sys: &mut dyn ParseSys, s: &CStr) -> c_int {
    let bytes = s.to_bytes_with_nul();
    let l = bytes.len();

    let (num, buf) = vm.string_table().alloc_string(l as c_int, sys);
    if buf.is_null() {
        return num;
    }

    let mut out = Vec::with_capacity(l);
    let mut i = 0;
    while i < l {
        if bytes[i] == b'\\' && i < l - 1 {
            i += 1;
            out.push(if bytes[i] == b'n' { b'\n' } else { b'\\' });
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }

    vm.write_engine_string(buf, &out);
    num
}

/// `ED_RezoneString` — replaces a zoned string reference, freeing the old one
/// if the `knownzone` bitmap says this engine allocated it.
pub fn ed_rezone_string(vm: &mut VmRaw, sys: &mut dyn ParseSys, reference: &mut c_int, s: &CStr) {
    if *reference != 0 {
        let id = (-1 - *reference) as usize;
        if vm.knownzone_test(id) {
            vm.knownzone_clear(id);
            // C frees the buffer PR_GetString resolves, *then* clears the slot
            let buf = vm.get_string(*reference).unwrap_or(core::ptr::null());
            vm.string_table().clear_engine_string(*reference, sys);
            sys.free(buf.cast_mut().cast::<u8>());
        }
    }

    let bytes = s.to_bytes_with_nul();
    let buf = sys.alloc(bytes.len()).cast::<core::ffi::c_char>();
    vm.write_engine_string(buf, bytes);
    *reference = vm.string_table().set_engine_string(buf.cast_const(), sys);

    let id = (-1 - *reference) as usize;
    vm.knownzone_grow_to(id, sys);
    vm.knownzone_set(id);
}

/// `ED_ParseEpair`. `dest` is the words at `base + key->ofs`, exactly as many
/// as the key's type occupies. Returns C's `qboolean`.
#[allow(clippy::too_many_arguments)]
pub fn ed_parse_epair(
    vm: &mut VmRaw,
    arena: &mut EdictArena,
    free_list: &mut FreeList,
    sys: &mut dyn ParseSys,
    dest: &mut [i32],
    key_type: c_int,
    key_s_name: c_int,
    s: &CStr,
    zoned: bool,
) -> Result<bool, ParseError> {
    match key_type & !c_int::from(DEF_SAVEGLOBAL) {
        etype::EV_STRING => {
            if zoned {
                // zoned version allows us to change the strings more freely
                let mut r = dest[0];
                ed_rezone_string(vm, sys, &mut r, s);
                dest[0] = r;
            } else {
                dest[0] = ed_new_string(vm, sys, s);
            }
        }

        etype::EV_FLOAT => dest[0] = (sys.atof(s) as f32).to_bits() as i32,
        // COMPAT: the 64-bit types are stored low word first. C stores
        // through `*(qcdouble_t *)d`, so on a big-endian target the order
        // would flip; every supported target (x86-64 and arm64 on Windows,
        // Linux and macOS) is little-endian, and the byte-diff gates only run
        // there. Asserted below so a big-endian port cannot land silently.
        etype::EV_EXT_DOUBLE => {
            let bits = sys.atof(s).to_bits();
            dest[0] = bits as u32 as i32;
            dest[1] = (bits >> 32) as u32 as i32;
        }
        etype::EV_EXT_INTEGER | etype::EV_EXT_UINT32 => {
            // COMPAT: C's ev_ext_uint32 arm uses atoi, not an unsigned parse
            dest[0] = sys.atoi(s);
        }
        etype::EV_EXT_SINT64 => {
            let v = sys.strtoll(s) as u64;
            dest[0] = v as u32 as i32;
            dest[1] = (v >> 32) as u32 as i32;
        }
        etype::EV_EXT_UINT64 => {
            let v = sys.strtoull(s);
            dest[0] = v as u32 as i32;
            dest[1] = (v >> 32) as u32 as i32;
        }

        etype::EV_VECTOR => {
            // COMPAT: C copies into a 128-byte buffer with q_strlcpy, so a
            // longer literal is truncated before parsing.
            let mut buf = [0u8; 128];
            let src = s.to_bytes();
            let n = src.len().min(buf.len() - 1);
            buf[..n].copy_from_slice(&src[..n]);
            let text = &buf[..n];

            let mut i = 0usize;
            let mut w = 0usize;
            // C walks a mutable copy, cutting at each space; `w <= end` lets
            // the final component start exactly at the terminator.
            while i < 3 && w <= n {
                let mut v = w;
                while v < n && text[v] != b' ' {
                    v += 1;
                }
                let field = &text[w..v];
                let c = std::ffi::CString::new(field).unwrap_or_default();
                dest[i] = (sys.atof(&c) as f32).to_bits() as i32;
                w = v + 1;
                i += 1;
            }
            if i < 3 {
                let name = vm.get_string_bytes(key_s_name).unwrap_or(b"");
                sys.dwarn(
                    "Avoided reading garbage for \"",
                    name,
                    "\" \"",
                    s.to_bytes(),
                    "\"\n",
                );
                for slot in dest.iter_mut().take(3).skip(i) {
                    *slot = 0.0f32.to_bits() as i32;
                }
            }
        }

        etype::EV_ENTITY => {
            // Spike: putentityfieldstring/etc should be able to cope with
            // etos's weirdness.
            let raw = s.to_bytes();
            let text = raw.strip_prefix(b"entity ").unwrap_or(raw);
            let c = std::ffi::CString::new(text).unwrap_or_default();
            let loaded = sys.atoi(&c);

            let max_edicts = vm.max_edicts();
            if loaded >= max_edicts {
                return Err(ParseError::EntityTooLarge {
                    num: loaded,
                    max_edicts,
                });
            }
            // COMPAT: C's own guard here is upper-bound only, but every
            // subsequent EDICT_NUM rejects `n < 0` too, so a negative literal
            // raises rather than addressing edict 0.
            if loaded < 0 {
                return Err(ParseError::BadEdictNum(loaded));
            }

            // loaded_ent_num can be beyond num_edicts at loading; adjust
            // first, because EDICT_NUM/ED_Free check against it.
            let previous = vm.num_edicts();
            vm.set_num_edicts(previous.max(loaded + 1));

            for j in previous..loaded {
                let id = EdictId(j as u32);
                // COMPAT (accepted divergence): C calls `EDICT_NUM (j)` here,
                // before the memset. Its release-build range test cannot fire
                // -- `j < loaded < max_edicts` and `j >= 0` both hold by
                // construction -- but under DEBUG/_DEBUG it also re-validates
                // the header fields this loop is about to overwrite, and that
                // consistency raise is not reproduced. Same shape as the
                // `EDICT_NUM (loaded_ent_num)` below.
                arena.clear_edict(id);
                arena.set_debug_header(id, vm.as_ptr().cast(), u64::from(id.0));
                debug_assert!(!arena.free(id));
                // C's ED_AddToFreeList raises here under DEBUG/_DEBUG; report
                // it so the raise happens in the C frame (ADR-009) instead of
                // silently wrapping the circular buffer.
                if quake_types::progs::ENGINE_DEBUG {
                    if let Some(kind) = alloc::free_list_would_overflow(free_list, max_edicts) {
                        return Err(ParseError::FreeListFull { kind, max_edicts });
                    }
                }
                alloc::ed_free(free_list, arena, id, vm.time(), &mut |e| {
                    sys.unlink_edict(e);
                });
            }

            let found = EdictId(loaded as u32);
            if arena.free(found) {
                alloc::remove_from_free_list(free_list, found);
                arena.set_free(found, false);
            }

            dest[0] = arena.to_prog(found);
        }

        etype::EV_FIELD => {
            let Some(ofs) = sys.find_field_ofs(s) else {
                // johnfitz -- HACK -- suppress error because fog/sky fields
                // might not be mentioned in defs.qc
                let b = s.to_bytes();
                if !b.starts_with(b"sky") && b != b"fog" {
                    sys.dprint("Can't find field ", b, "\n");
                }
                return Ok(false);
            };
            dest[0] = vm.g_i32(ofs as usize);
        }

        etype::EV_FUNCTION => {
            let Some(index) = sys.find_function(s) else {
                sys.print("Can't find function ", s.to_bytes(), "\n");
                return Ok(false);
            };
            dest[0] = index;
        }

        _ => {}
    }
    Ok(true)
}
