//! `vid.h` `vrect_t` and `render.h` `refdef_t` -- ADR-011 mirrors of the
//! `r_refdef` view rectangle/fov block. They embed no Vulkan handle, so they
//! stay outside the `render` feature: `quake-capi::view` (host) and
//! `quake-capi::gl_screen` (render) both read `r_refdef` through them
//! (Rust migration Phase 8 M7).

use core::ffi::{c_float, c_int};

/// `vid.h:47-53` -- `vrect_t`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VRect {
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
    pub pnext: *mut VRect,
}

/// `render.h` `refdef_t` -- the global `r_refdef`, owned by `gl_rmain.c`
/// (`SCR_CalcRefdef` in `quake-capi::gl_screen` fills the vrect/fov part).
#[repr(C)]
pub struct RefDef {
    pub vrect: VRect,
    pub aliasvrect: VRect,
    pub vrectright: c_int,
    pub vrectbottom: c_int,
    pub aliasvrectright: c_int,
    pub aliasvrectbottom: c_int,
    pub vrectrightedge: c_float,
    pub fvrectx: c_float,
    pub fvrecty: c_float,
    pub fvrectx_adj: c_float,
    pub fvrecty_adj: c_float,
    pub vrect_x_adj_shift20: c_int,
    pub vrectright_adj_shift20: c_int,
    pub fvrectright_adj: c_float,
    pub fvrectbottom_adj: c_float,
    pub fvrectright: c_float,
    pub fvrectbottom: c_float,
    pub horizontal_field_of_view: c_float,
    pub x_origin: c_float,
    pub y_origin: c_float,
    pub vieworg: [c_float; 3],
    pub viewangles: [c_float; 3],
    pub basefov: c_float,
    pub fov_x: c_float,
    pub fov_y: c_float,
    pub ambientlight: c_int,
}
