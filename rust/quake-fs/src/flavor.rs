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

/// C: `COM_IsValidFlavorDir` over pre-probed existence: classic needs
/// `<dir>/id1/pak0.pak`, remastered needs `<dir>/QuakeEX.kpf`; no
/// preference accepts either. A path that overflowed MAX_OSPATH during
/// assembly counts as absent (the shim passes false).
pub fn is_valid_flavor_dir(
    flavor: FlavorRequest,
    classic_pak_exists: bool,
    kpf_exists: bool,
) -> bool {
    if flavor != FlavorRequest::Remastered && classic_pak_exists {
        return true;
    }
    if flavor != FlavorRequest::Original && kpf_exists {
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
        assert!(is_valid_flavor_dir(Original, true, false));
        assert!(!is_valid_flavor_dir(Original, false, true));
        assert!(is_valid_flavor_dir(Remastered, false, true));
        assert!(!is_valid_flavor_dir(Remastered, true, false));
        assert!(is_valid_flavor_dir(NoPreference, true, false));
        assert!(is_valid_flavor_dir(NoPreference, false, true));
        assert!(!is_valid_flavor_dir(NoPreference, false, false));
    }
}
