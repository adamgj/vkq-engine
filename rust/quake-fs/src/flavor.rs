//! Quake flavor selection decisions (original vs 2021 rerelease). The
//! filesystem probes and the command-line parm lookups stay in the FFI
//! shim; the decision tables live here.

/// C: `quakeflavor_t` (steam.h); `NoPreference` is COM_RequestedQuakeFlavor's
/// -1 "none requested".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlavorRequest {
    Original,
    Remastered,
    NoPreference,
}

/// C: `COM_RequestedQuakeFlavor` — remaster parms win over original parms.
pub fn requested_flavor(has_remaster_parm: bool, has_original_parm: bool) -> FlavorRequest {
    if has_remaster_parm {
        FlavorRequest::Remastered
    } else if has_original_parm {
        FlavorRequest::Original
    } else {
        FlavorRequest::NoPreference
    }
}

/// C: `COM_IsValidFlavorDir` — classic needs `<dir>/id1/pak0.pak`,
/// remastered needs `<dir>/QuakeEX.kpf`; no preference accepts either.
///
/// The probes are injected rather than pre-computed so this keeps the C's
/// short-circuit: a classic hit returns without ever stat'ing the kpf, and
/// a remastered request never stats the pak. Each closure reports whether
/// its file exists; a path that overflowed MAX_OSPATH during assembly
/// counts as absent (the caller returns false without stat'ing).
pub fn is_valid_flavor_dir(
    flavor: FlavorRequest,
    classic_pak_exists: impl FnOnce() -> bool,
    kpf_exists: impl FnOnce() -> bool,
) -> bool {
    if flavor != FlavorRequest::Remastered && classic_pak_exists() {
        return true;
    }
    if flavor != FlavorRequest::Original && kpf_exists() {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_precedence() {
        assert_eq!(requested_flavor(true, true), FlavorRequest::Remastered);
        assert_eq!(requested_flavor(true, false), FlavorRequest::Remastered);
        assert_eq!(requested_flavor(false, true), FlavorRequest::Original);
        assert_eq!(requested_flavor(false, false), FlavorRequest::NoPreference);
    }

    #[test]
    fn flavor_dir_validity() {
        use FlavorRequest::*;
        let valid = |flavor, classic, kpf| is_valid_flavor_dir(flavor, || classic, || kpf);
        assert!(valid(Original, true, false));
        assert!(!valid(Original, false, true));
        assert!(valid(Remastered, false, true));
        assert!(!valid(Remastered, true, false));
        assert!(valid(NoPreference, true, false));
        assert!(valid(NoPreference, false, true));
        assert!(!valid(NoPreference, false, false));
    }

    /// The C is `if (flavor != REMASTERED && ... pak0) return true; if
    /// (flavor != ORIGINAL && ... kpf) return true;` — each probe is a
    /// Sys_FileType call, so a probe that the C never makes must not be
    /// made here either.
    #[test]
    fn probes_short_circuit_like_the_c() {
        use core::cell::Cell;
        use FlavorRequest::*;

        let run = |flavor, classic: bool, kpf: bool| {
            let (nc, nk) = (Cell::new(0u32), Cell::new(0u32));
            let out = is_valid_flavor_dir(
                flavor,
                || {
                    nc.set(nc.get() + 1);
                    classic
                },
                || {
                    nk.set(nk.get() + 1);
                    kpf
                },
            );
            (out, nc.get(), nk.get())
        };

        // classic hit: the kpf is never stat'ed
        assert_eq!(run(NoPreference, true, true), (true, 1, 0));
        assert_eq!(run(Original, true, true), (true, 1, 0));
        // remastered request skips the pak probe entirely
        assert_eq!(run(Remastered, true, true), (true, 0, 1));
        // original request never looks at the kpf, even on a miss
        assert_eq!(run(Original, false, true), (false, 1, 0));
        // no preference, classic miss: both get probed, in that order
        assert_eq!(run(NoPreference, false, true), (true, 1, 1));
        assert_eq!(run(NoPreference, false, false), (false, 1, 1));
    }
}
