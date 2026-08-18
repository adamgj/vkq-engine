//! ADR-005 exhaustive conformance sweep: every one of the 2^32 f32 bit
//! patterns, formatted with the engine's float specs, compared against the
//! platform snprintf. Scheduled weekly per CI OS; run locally with:
//!
//!     cargo run -p quake-ctest --release --bin snprintf_sweep -- --exhaustive
//!
//! Without --exhaustive a 2^24-pattern subset runs (stride sampling), as a
//! quick smoke.

use quake_ctest::c_snprintf_f;
use quake_util::printf::{format, Arg};

// %f is what savegames write; % 7.1f / % 5.0f are the console float shapes
const SPECS: &[&str] = &["%f", "% 7.1f", "% 5.0f  "];

fn check(bits: u32) -> Result<(), String> {
    let v = f32::from_bits(bits) as f64;
    for spec in SPECS {
        let c = c_snprintf_f(spec, v);
        let r = String::from_utf8(format(spec.as_bytes(), &[Arg::F64(v)])).unwrap();
        if r != c {
            return Err(format!(
                "bits {bits:#010x} value {v:?} spec {spec:?}: rust {r:?} != c {c:?}"
            ));
        }
    }
    Ok(())
}

fn main() {
    let exhaustive = std::env::args().any(|a| a == "--exhaustive");
    let stride: u64 = if exhaustive { 1 } else { 256 };
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get() as u64);

    let total: u64 = 1 << 32;
    let chunk = total / threads + 1;
    let mismatches: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    let mut errs = Vec::new();
                    let start = t * chunk;
                    let end = (start + chunk).min(total);
                    let mut b = start;
                    while b < end {
                        if let Err(e) = check(b as u32) {
                            if errs.len() < 20 {
                                errs.push(e);
                            }
                        }
                        b += stride;
                    }
                    errs
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });

    let checked = total / stride;
    if mismatches.is_empty() {
        println!(
            "OK: {checked} f32 patterns x {} specs match platform snprintf",
            SPECS.len()
        );
    } else {
        eprintln!(
            "{} mismatching patterns (first 20 per thread):",
            mismatches.len()
        );
        for m in &mismatches {
            eprintln!("  {m}");
        }
        std::process::exit(1);
    }
}
