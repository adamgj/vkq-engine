//! `mplane_t` mirror (`Quake/gl_model.h`). Compat-critical ABI: BoxOnPlaneSide
//! reads `normal`, `dist` and `signbits` through this layout from C-owned
//! memory (ADR-011: hand-written mirror + const layout asserts).

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MPlane {
    pub normal: [f32; 3],
    pub dist: f32,
    /// for texture axis selection and fast side tests
    pub type_: u8,
    /// signx + signy<<1 + signz<<2
    pub signbits: u8,
    pub pad: [u8; 2],
}

const _: () = assert!(std::mem::size_of::<MPlane>() == 20);
const _: () = assert!(std::mem::offset_of!(MPlane, normal) == 0);
const _: () = assert!(std::mem::offset_of!(MPlane, dist) == 12);
const _: () = assert!(std::mem::offset_of!(MPlane, type_) == 16);
const _: () = assert!(std::mem::offset_of!(MPlane, signbits) == 17);
const _: () = assert!(std::mem::offset_of!(MPlane, pad) == 18);
