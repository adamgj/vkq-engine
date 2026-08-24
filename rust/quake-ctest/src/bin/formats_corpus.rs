//! Phase 3 M7 (D9): real-asset corpus gate over the model/image seam.
//!
//! Mounts one game directory on both filesystems (c_ref C and Rust), loads
//! every asset named in the list file through both sides' loaders via
//! `quake_ctest::drivers`, asserts the observation streams are identical,
//! and emits one manifest line per asset: `OK <fnv1a64> <path>` for parity
//! over an accepted asset, `REJECT <path> <message>` when the C oracle
//! refuses it (the Rust side is not run: its Sys_Error aborts), and
//! `SKIP <path> <why>` for files outside the gate's scope. Per ADR-019 only
//! hashes leave the process — no game data is copied or embedded.
//!
//! Driven by `scripts/harness/run_formats_corpus.py`, which discovers the
//! assets (loose files plus pak directories) and diffs manifests between
//! runs or platforms.
//!
//! Exit status: 0 when every listed asset held C/Rust parity *and* at
//! least one produced a parity-verified OK (an all-SKIP run — the signature
//! of a broken mount — exits 1); a parity divergence panics with a
//! line-level diff.

use std::ffi::CString;
use std::io::Read;

use quake_ctest::drivers as drv;
use quake_ctest::fs::{self as ctfs, Side};
use quake_ctest::model_hash::{BlobLens, Snapshot};
use quake_types::bspfile::{
    BSP2VERSION_2PSB, BSP2VERSION_BSP2, BSPVERSION, BSPVERSION_QUAKE64, BSPVERSION_VALVE,
    LUMP_VISIBILITY,
};

fn fnv1a64(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

struct Hasher(u64);

impl Hasher {
    fn new() -> Self {
        Hasher(0xcbf2_9ce4_8422_2325)
    }
    fn lines(&mut self, lines: &[String]) {
        for l in lines {
            fnv1a64(&mut self.0, l.as_bytes());
            fnv1a64(&mut self.0, b"\n");
        }
    }
    fn debug<T: std::fmt::Debug>(&mut self, v: &T) {
        fnv1a64(&mut self.0, format!("{v:?}").as_bytes());
    }
    fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

fn compare_snaps(what: &str, c: &[Snapshot], r: &[Snapshot]) {
    assert_eq!(c.len(), r.len(), "{what}: model count");
    for (i, (a, b)) in c.iter().zip(r.iter()).enumerate() {
        a.assert_eq(b, &format!("{what} model {i}"));
    }
}

enum Outcome {
    Ok(String),
    Reject(String),
    Skip(&'static str),
}

fn run_bsp(name: &str) -> Outcome {
    let cname = CString::new(name).unwrap();
    let Some((data, _, _, path_id)) = ctfs::load_file(Side::C, &cname) else {
        return Outcome::Skip("unreadable");
    };
    if data.len() < 4 + 15 * 8 {
        return Outcome::Skip("short header");
    }
    let version = i32::from_le_bytes(data[..4].try_into().unwrap());
    if ![
        BSPVERSION,
        BSPVERSION_VALVE,
        BSPVERSION_QUAKE64,
        BSP2VERSION_2PSB,
        BSP2VERSION_BSP2,
    ]
    .contains(&version)
    {
        // Mod_LoadBrushModel (C orchestrator) Sys_Errors before the seam
        return Outcome::Skip("unknown bsp version");
    }
    let vis = drv::lump_of(&data, LUMP_VISIBILITY);
    let light = drv::lump_of(&data, quake_types::bspfile::LUMP_LIGHTING);
    // every lump descriptor must lie inside the file: the seam loaders trust
    // them (the engine's orchestrator does too — a shipped map is in-domain,
    // a truncated one is not worth crashing the gate over)
    for i in 0..15 {
        let l = drv::lump_of(&data, i);
        if l.fileofs < 0
            || l.filelen < 0
            || (l.fileofs as i64 + l.filelen as i64) > data.len() as i64
        {
            return Outcome::Skip("lump outside file");
        }
    }
    let lens_for = |side: Side| BlobLens {
        visdata: vis.filelen.max(0) as usize,
        lightdata: drv::lit_replacement_len(side, name, path_id, light.filelen)
            .unwrap_or_else(|| drv::expanded_light_len(version, light.filelen)),
    };
    // sv_modelname == mod->name makes every map the world model, so
    // Mod_SetupSubmodels' clipbox-skip branch runs for submodel 0 while the
    // i > 0 clones still take the copy — both arms in one pass (RA13). The
    // real name also routes the `.lit`/`.ent` sidecar probes (AC4).
    let sv_modelname = cname.clone();
    // SAFETY: the descriptors were bounds-checked above and the asset is a
    // shipped map (in the loaders' defined domain); C runs first, so a
    // C-side fatal is trapped before the Rust side ever runs
    let c =
        unsafe { drv::bsp_load_side(Side::C, name, &data, &sv_modelname, 1.0, lens_for(Side::C)) };
    if let Some(drv::LoadError::Sys(msg)) = c.error {
        return Outcome::Reject(msg);
    }
    // SAFETY: as above; the C side accepted, establishing the decision
    let r = unsafe {
        drv::bsp_load_side(
            Side::Rust,
            name,
            &data,
            &sv_modelname,
            1.0,
            lens_for(Side::Rust),
        )
    };
    assert_eq!(c.error, r.error, "{name}: error parity");
    assert_eq!(c.con_log, r.con_log, "{name}: console log parity");
    compare_snaps(name, &c.snaps, &r.snaps);
    let mut h = Hasher::new();
    for s in &c.snaps {
        h.lines(&s.lines);
    }
    h.lines(&c.con_log);
    h.debug(&c.error);
    Outcome::Ok(h.hex())
}

fn run_mdl(name: &str) -> Outcome {
    let cname = CString::new(name).unwrap();
    let Some((data, _, _, _)) = ctfs::load_file(Side::C, &cname) else {
        return Outcome::Skip("unreadable");
    };
    // SAFETY: shipped .mdl files are in the parser's domain; C runs first
    // and a fatal is trapped, in which case the Rust side is skipped
    let c = unsafe { drv::alias_load_side(Side::C, name, &data, true) };
    if let Some(msg) = c.error {
        return Outcome::Reject(msg);
    }
    // SAFETY: as above — C accepted
    let r = unsafe { drv::alias_load_side(Side::Rust, name, &data, true) };
    assert_eq!(c.con_log, r.con_log, "{name}: console log parity");
    assert_eq!(c.skins, r.skins, "{name}: Mod_LoadAllSkins argument parity");
    let (cs, rs) = (c.snap.unwrap(), r.snap.unwrap());
    cs.assert_eq(&rs, name);
    let mut h = Hasher::new();
    h.lines(&cs.lines);
    h.lines(&c.con_log);
    h.debug(&c.skins);
    Outcome::Ok(h.hex())
}

fn run_spr(name: &str) -> Outcome {
    let cname = CString::new(name).unwrap();
    let Some((data, _, _, _)) = ctfs::load_file(Side::C, &cname) else {
        return Outcome::Skip("unreadable");
    };
    // SAFETY: shipped .spr files are in the parser's domain; C-first
    let c = unsafe { drv::sprite_load_side(Side::C, name, &data) };
    if let Some(msg) = c.error {
        return Outcome::Reject(msg);
    }
    // SAFETY: as above — C accepted
    let r = unsafe { drv::sprite_load_side(Side::Rust, name, &data) };
    assert_eq!(c.con_log, r.con_log, "{name}: console log parity");
    assert_eq!(c.textures, r.textures, "{name}: TexMgr_LoadImage parity");
    let (cs, rs) = (c.snap.unwrap(), r.snap.unwrap());
    cs.assert_eq(&rs, name);
    let mut h = Hasher::new();
    h.lines(&cs.lines);
    h.lines(&c.con_log);
    h.debug(&c.textures);
    Outcome::Ok(h.hex())
}

fn run_md3(name: &str) -> Outcome {
    let cname = CString::new(name).unwrap();
    let Some((data, _, _, _)) = ctfs::load_file(Side::C, &cname) else {
        return Outcome::Skip("unreadable");
    };
    // SAFETY: shipped .md3 files are in the parser's domain; C-first
    let c = unsafe { drv::md3_load_side(Side::C, name, &data, 0) };
    if let Some(msg) = c.error {
        return Outcome::Reject(msg);
    }
    // SAFETY: as above — C accepted
    let r = unsafe { drv::md3_load_side(Side::Rust, name, &data, 0) };
    assert_eq!(c.con_log, r.con_log, "{name}: console log parity");
    assert_eq!(c.skins, r.skins, "{name}: skin-callback parity");
    assert_eq!(c.uploads, r.uploads, "{name}: GLMesh_UploadBuffers parity");
    assert_eq!(c.image, r.image, "{name}: in-place mutation parity");
    let (cs, rs) = (c.snap.unwrap(), r.snap.unwrap());
    cs.assert_eq(&rs, name);
    let mut h = Hasher::new();
    h.lines(&cs.lines);
    h.lines(&c.con_log);
    h.debug(&c.uploads.len());
    for u in &c.uploads {
        h.debug(&u.hashes);
    }
    Outcome::Ok(h.hex())
}

fn run_md5(name: &str) -> Outcome {
    let cname = CString::new(name).unwrap();
    let Some((data, _, _, _)) = ctfs::load_file(Side::C, &cname) else {
        return Outcome::Skip("unreadable");
    };
    // SAFETY: MD5 is recoverable (both sides return ok=false on rejects);
    // the model name routes the companion .md5anim through the mounted fs
    let c = unsafe { drv::md5_load_side(Side::C, name, &data, 0) };
    // SAFETY: as above
    let r = unsafe { drv::md5_load_side(Side::Rust, name, &data, 0) };
    assert_eq!(c.ok, r.ok, "{name}: return value parity");
    assert_eq!(c.con_log, r.con_log, "{name}: console log parity");
    assert_eq!(c.skins, r.skins, "{name}: skin-callback parity");
    assert_eq!(c.uploads, r.uploads, "{name}: GLMesh_UploadBuffers parity");
    c.snap.assert_eq(&r.snap, name);
    if !c.ok {
        return Outcome::Reject("recoverable MD5 failure (parity held)".into());
    }
    let mut h = Hasher::new();
    h.lines(&c.snap.lines);
    h.lines(&c.con_log);
    for u in &c.uploads {
        h.debug(&u.hashes);
    }
    Outcome::Ok(h.hex())
}

fn run_image(name: &str, pcx: bool) -> Outcome {
    let cname = CString::new(name).unwrap();
    if ctfs::load_file(Side::C, &cname).is_none() {
        return Outcome::Skip("unreadable");
    }
    let buf_len = |w: i32, h: i32| -> usize {
        if pcx {
            (w.wrapping_mul(h).wrapping_add(1).wrapping_mul(4)).max(0) as usize
        } else {
            (w as u32).wrapping_mul(h as u32) as usize
        }
    };
    // SAFETY: the fixture opens (checked above); C runs under the trap
    let c = unsafe { drv::image_decode_side(Side::C, &cname, pcx, buf_len) };
    if let Some(msg) = c.error {
        return Outcome::Reject(msg);
    }
    // SAFETY: C accepted, establishing the decision
    let r = unsafe { drv::image_decode_side(Side::Rust, &cname, pcx, buf_len) };
    assert_eq!(c, r, "{name}: decode parity");
    assert_eq!(c.open_handles, 0, "{name}: decoder must close the handle");
    let mut h = Hasher::new();
    h.debug(&(c.width, c.height));
    if let Some(d) = &c.data {
        fnv1a64(&mut h.0, d);
    }
    h.lines(&c.con_log);
    Outcome::Ok(h.hex())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut base = None;
    let mut gamedir = None;
    let mut list = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--base" => base = args.next(),
            "--gamedir" => gamedir = args.next(),
            "--list" => list = args.next(),
            other => panic!("unknown argument {other}"),
        }
    }
    let base = base.expect("--base <dir> required");
    let gamedir = gamedir.expect("--gamedir <name> required");
    let names: Vec<String> = match list {
        Some(f) => std::fs::read_to_string(f).expect("readable --list file"),
        None => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).expect("stdin");
            s
        }
    }
    .lines()
    .map(str::trim)
    .filter(|l| !l.is_empty() && !l.starts_with('#'))
    .map(String::from)
    .collect();

    let _guard = ctfs::lock();
    let gd = CString::new(gamedir.clone()).unwrap();
    for side in ctfs::BOTH {
        ctfs::setup(side, &[std::path::Path::new(&base)], 0, &gd);
    }

    let (mut ok, mut reject, mut skip) = (0u32, 0u32, 0u32);
    for name in &names {
        let lower = name.to_ascii_lowercase();
        let outcome = if lower.ends_with(".bsp") {
            run_bsp(name)
        } else if lower.ends_with(".mdl") {
            run_mdl(name)
        } else if lower.ends_with(".spr") {
            run_spr(name)
        } else if lower.ends_with(".md3") {
            run_md3(name)
        } else if lower.ends_with(".md5mesh") {
            run_md5(name)
        } else if lower.ends_with(".pcx") {
            run_image(name, true)
        } else if lower.ends_with(".lmp") {
            run_image(name, false)
        } else {
            Outcome::Skip("unknown extension")
        };
        match outcome {
            Outcome::Ok(h) => {
                ok += 1;
                println!("OK {h} {name}");
            }
            Outcome::Reject(msg) => {
                reject += 1;
                println!("REJECT {name} {msg}");
            }
            Outcome::Skip(why) => {
                skip += 1;
                println!("SKIP {name} {why}");
            }
        }
    }
    println!(
        "# {ok} ok, {reject} reject, {skip} skip, {} total",
        names.len()
    );
    // a broken mount makes every asset SKIP("unreadable"), which must not
    // be indistinguishable from success at the exit-code level
    if !names.is_empty() && ok == 0 {
        eprintln!("formats_corpus: no asset produced a parity-verified OK outcome");
        std::process::exit(1);
    }
}
