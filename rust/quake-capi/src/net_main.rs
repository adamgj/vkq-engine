//! Phase 5 M9 / Phase 7 M9c: net_main.c's logic in Rust -- qsocket pool
//! management, SetNetTime, the qsocket accessors, the slist UI helpers, the
//! listen/maxplayers/port command handlers, the leaf driver loops (Close,
//! CheckNewConnections, ListAddresses) and, since M9c, the fourteen dispatch
//! funnels. net_main.c keeps trampolines under `USE_RUST_NET` (the Phase 3
//! in-file idiom).
//!
//! ADR-009: `Host_Error`-capable code runs beneath the funnels (the dgrm
//! glue's re-raise, `_Datagram_ServerControlPacket` -> SV_ConnectClient, the
//! MSG-writer glue under SearchForHosts), and a longjmp must never unwind a
//! Rust frame. M9c therefore pushed the required C frame down to the
//! individual raise-capable vtable calls -- `NetMain_Glue_QGetMessage`,
//! `NetMain_Glue_QGetAnyMessage`, `NetMain_Glue_DriverConnect`,
//! `NetMain_Glue_DriverInit`, `NetMain_Glue_RegisterNetVars` -- instead of
//! keeping whole funnels in C. The five cores that can observe a caught jump
//! return the `Host_Guard` status untouched; their C wrapper re-issues it
//! with `Host_Reraise`.
//!
//! Ownership (ADR-007): pool heads, hostcache, counters and net_time stay
//! C-owned in net_main.c; `slistLastShown`, `slistStartTime`,
//! `slistActiveTime`, `slistSendProcedure`, `slistPollProcedure` and the
//! `pollProcedureList` *head* are Rust module state. The PollProcedure nodes
//! themselves stay caller-owned, exactly as in C.

use core::ffi::{c_char, c_int, c_void, CStr};

use quake_c_sys as c;
use quake_c_sys::host::HOST_GUARD_OK;
use quake_net::cnum::c_atoi;
use quake_types::net::{
    HostCache, NetDriver, PollProcedure, QBoolean, QHostAddr, QSocket, SizeBuf, HOSTCACHESIZE,
    NET_MAXMESSAGE,
};

extern "C" {
    static net_numdrivers: c_int;
    /// `hostcache_t hostcache[HOSTCACHESIZE]` (net_defs.h) -- a complete
    /// array type in C, so the extern can describe it truthfully
    static mut hostcache: [HostCache; HOSTCACHESIZE];
}

/// `net_drivers[idx]`. See `net_dgrm::landriver`: the C array is an
/// incomplete type, so the base pointer comes from C to give the offset
/// provenance over the real object rather than over a fabricated one.
fn driver(idx: c_int) -> *mut NetDriver {
    // SAFETY: every call site bounds idx by net_numdrivers, which is the
    // count that sized the array NetMain_Drivers returns
    unsafe { c::NetMain_Drivers().cast::<NetDriver>().add(idx as usize) }
}

fn host(idx: usize) -> *mut HostCache {
    // SAFETY: every call site bounds idx by hostCacheCount, which net_dgrm.c
    // never lets exceed HOSTCACHESIZE -- the declared extent of the array
    unsafe { (&raw mut hostcache).cast::<HostCache>().add(idx) }
}

fn con_print(text: &str) {
    let mut b = text.as_bytes().to_vec();
    b.push(0);
    // SAFETY: b is NUL-terminated
    unsafe {
        c::Con_Printf(c"%s".as_ptr(), b.as_ptr());
    }
}

fn c_str_field(dst: &mut [c_char], s: &str) {
    let b = s.as_bytes();
    for (i, &ch) in b.iter().enumerate() {
        dst[i] = ch as c_char;
    }
    dst[b.len()] = 0;
}

fn field_bytes(src: &[c_char]) -> Vec<u8> {
    src.iter()
        .take_while(|&&ch| ch != 0)
        .map(|&ch| ch as u8)
        .collect()
}

/// C's `%-W.Ws`: truncate to W bytes, pad to W with spaces (byte-exact --
/// quake names can carry arbitrary high-bit bytes)
fn pad_trunc(src: &[c_char], width: usize) -> Vec<u8> {
    let mut b = field_bytes(src);
    b.truncate(width);
    while b.len() < width {
        b.push(b' ');
    }
    b
}

fn con_print_bytes(text: &[u8]) {
    let mut b = text.to_vec();
    b.push(0);
    // SAFETY: b is NUL-terminated
    unsafe {
        c::Con_Printf(c"%s".as_ptr(), b.as_ptr());
    }
}

/// `SetNetTime`
#[no_mangle]
pub extern "C" fn rust_net_SetNetTime() -> f64 {
    // SAFETY: host-thread-only globals
    unsafe {
        c::net_time = c::Sys_DoubleTime();
        c::net_time
    }
}

/// `NET_NewQSocket`
///
/// # Safety
/// Single-threaded host frame; the pool was built by NET_Init.
#[no_mangle]
pub unsafe extern "C" fn rust_net_NewQSocket() -> *mut QSocket {
    // SAFETY: caller contract
    unsafe {
        if c::net_freeSockets.is_null() {
            return core::ptr::null_mut();
        }
        if c::net_activeconnections >= c::NetMain_MaxClients() {
            return core::ptr::null_mut();
        }

        // get one from free list
        let sock = c::net_freeSockets.cast::<QSocket>();
        c::net_freeSockets = (*sock).next.cast();

        // add it to active list
        (*sock).next = c::net_activeSockets.cast();
        c::net_activeSockets = sock.cast();

        let s = &mut *sock;
        s.isvirtual = false;
        s.disconnected = false;
        s.connecttime = c::net_time;
        c_str_field(&mut s.trueaddress, "UNSET ADDRESS");
        c_str_field(&mut s.maskedaddress, "UNSET ADDRESS");
        s.driver = c::net_driverlevel;
        s.socket = 0;
        s.driverdata = core::ptr::null_mut();
        s.can_send = true;
        s.send_next = false;
        s.last_message_time = c::net_time;
        s.ack_sequence = 0;
        s.send_sequence = 0;
        s.unreliable_send_sequence = 0;
        s.send_message_length = 0;
        s.receive_sequence = 0;
        s.unreliable_receive_sequence = 0;
        s.receive_message_length = 0;
        s.pending_max_datagram = 1024;
        s.proquake_angle_hack = false;

        sock
    }
}

/// `NET_FreeQSocket`
///
/// # Safety
/// `sock` is a pool socket; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_FreeQSocket(sock: *mut QSocket) {
    // SAFETY: caller contract
    unsafe {
        // remove it from active list
        if sock == c::net_activeSockets.cast() {
            c::net_activeSockets = (*sock).next.cast();
        } else {
            let mut s = c::net_activeSockets.cast::<QSocket>();
            loop {
                if s.is_null() {
                    c::Sys_Error(c"NET_FreeQSocket: not active".as_ptr());
                }
                if (*s).next == sock {
                    (*s).next = (*sock).next;
                    break;
                }
                s = (*s).next;
            }
        }

        // add it to free list
        (*sock).next = c::net_freeSockets.cast();
        c::net_freeSockets = sock.cast();
        (*sock).disconnected = true;
    }
}

/// # Safety
/// `s` is a live qsocket.
#[no_mangle]
pub unsafe extern "C" fn rust_net_QSocketGetSequenceIn(s: *const QSocket) -> c_int {
    // returns the last unreliable sequence that was received
    // SAFETY: caller contract
    unsafe { (*s).unreliable_receive_sequence.wrapping_sub(1) as c_int }
}

/// # Safety
/// `s` is a live qsocket.
#[no_mangle]
pub unsafe extern "C" fn rust_net_QSocketGetSequenceOut(s: *const QSocket) -> c_int {
    // returns the next unreliable sequence that will be sent
    // SAFETY: caller contract
    unsafe { (*s).unreliable_send_sequence as c_int }
}

/// # Safety
/// `s` is a live qsocket.
#[no_mangle]
pub unsafe extern "C" fn rust_net_QSocketGetTime(s: *const QSocket) -> f64 {
    // SAFETY: caller contract
    unsafe { (*s).connecttime }
}

/// # Safety
/// `s` is a live qsocket; the returned pointer is into the qsocket.
#[no_mangle]
pub unsafe extern "C" fn rust_net_QSocketGetTrueAddressString(s: *const QSocket) -> *const c_char {
    // SAFETY: caller contract
    unsafe { (*s).trueaddress.as_ptr() }
}

/// # Safety
/// `s` is a live qsocket; the returned pointer is into the qsocket.
#[no_mangle]
pub unsafe extern "C" fn rust_net_QSocketGetMaskedAddressString(
    s: *const QSocket,
) -> *const c_char {
    // SAFETY: caller contract
    unsafe { (*s).maskedaddress.as_ptr() }
}

/// # Safety
/// `s` may be null or disconnected (demo playback).
#[no_mangle]
pub unsafe extern "C" fn rust_net_QSocketGetProQuakeAngleHack(s: *const QSocket) -> bool {
    // SAFETY: caller contract
    unsafe {
        if !s.is_null() && !(*s).disconnected {
            (*s).proquake_angle_hack
        } else {
            false // happens with demos
        }
    }
}

/// # Safety
/// `s` is a live qsocket.
#[no_mangle]
pub unsafe extern "C" fn rust_net_QSocketSetMSS(s: *mut QSocket, mss: c_int) {
    // SAFETY: caller contract
    unsafe {
        (*s).pending_max_datagram = mss;
    }
}

fn argv(i: c_int) -> Vec<u8> {
    // SAFETY: Cmd_Argv returns a NUL-terminated string valid for the command
    unsafe { CStr::from_ptr(c::Cmd_Argv(i)).to_bytes().to_vec() }
}

/// `NET_Listen_f`
///
/// # Safety
/// Command context; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_Listen_f() {
    // SAFETY: caller contract
    unsafe {
        if c::Cmd_Argc() != 2 {
            con_print(&format!(
                "\"listen\" is \"{}\"\n",
                if c::listening { 1 } else { 0 }
            ));
            return;
        }

        c::listening = c_atoi(&argv(1)) != 0;

        c::net_driverlevel = 0;
        while c::net_driverlevel < net_numdrivers {
            let d = driver(c::net_driverlevel);
            if (*d).initialized {
                ((*d).listen.expect("driver Listen"))(c::listening);
            }
            c::net_driverlevel += 1;
        }
    }
}

/// `MaxPlayers_f`
///
/// # Safety
/// Command context; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_MaxPlayers_f() {
    // SAFETY: caller contract
    unsafe {
        if c::Cmd_Argc() != 2 {
            con_print(&format!(
                "\"maxplayers\" is \"{}\"\n",
                c::NetMain_MaxClients()
            ));
            return;
        }

        if c::NetMain_SVActive() {
            con_print("maxplayers can not be changed while a server is running.\n");
            return;
        }

        let mut n = c_atoi(&argv(1));
        if n < 1 {
            n = 1;
        }
        if n > c::NetMain_MaxClientsLimit() {
            n = c::NetMain_MaxClientsLimit();
            con_print(&format!("\"maxplayers\" set to \"{n}\"\n"));
        }

        if n == 1 && c::listening {
            c::Cbuf_AddText(c"listen 0\n".as_ptr());
        }
        if n > 1 && !c::listening {
            c::Cbuf_AddText(c"listen 1\n".as_ptr());
        }

        c::NetMain_SetMaxClients(n);
        if n == 1 {
            c::Cvar_Set(c"deathmatch".as_ptr(), c"0".as_ptr());
        } else {
            c::Cvar_Set(c"deathmatch".as_ptr(), c"1".as_ptr());
        }
    }
}

/// `NET_Port_f`
///
/// # Safety
/// Command context; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_Port_f() {
    // SAFETY: caller contract
    unsafe {
        if c::Cmd_Argc() != 2 {
            let hp = c::net_hostport;
            con_print(&format!("\"port\" is \"{hp}\"\n"));
            return;
        }

        let n = c_atoi(&argv(1));
        if !(1..=65534).contains(&n) {
            con_print("Bad value, must be between 1 and 65534\n");
            return;
        }

        c::DEFAULTnet_hostport = n;
        c::net_hostport = n;

        if c::listening {
            // force a change to the new port
            c::Cbuf_AddText(c"listen 0\n".as_ptr());
            c::Cbuf_AddText(c"listen 1\n".as_ptr());
        }
    }
}

/// `slistLastShown` (C file static, touched only by these print helpers)
static mut SLIST_LAST_SHOWN: usize = 0;

/// `PrintSlistHeader`
#[no_mangle]
pub extern "C" fn rust_net_PrintSlistHeader() {
    con_print("Server          Map             Users\n");
    con_print("--------------- --------------- -----\n");
    // SAFETY: host-thread-only module static
    unsafe {
        SLIST_LAST_SHOWN = 0;
    }
}

/// `PrintSlist`
///
/// # Safety
/// Single-threaded host frame (hostcache is C-owned).
#[no_mangle]
pub unsafe extern "C" fn rust_net_PrintSlist() {
    // SAFETY: caller contract
    unsafe {
        let mut n = SLIST_LAST_SHOWN;
        while n < c::hostCacheCount {
            let h = &*host(n);
            let mut line = pad_trunc(&h.name, 15);
            line.push(b' ');
            line.extend_from_slice(&pad_trunc(&h.map, 15));
            if h.maxusers != 0 {
                // COMPAT: C's format is `%2u` applied to an `int`. Both
                // fields are remote-controlled and can be negative --
                // net_dgrm.c assigns MSG_ReadByte() (-1 on badread, i.e. a
                // truncated CCREP_SERVER_INFO) and atoi() of a dpmaster
                // `clients` key -- so the unsigned reinterpretation is
                // observable: users == -1 prints 4294967295, not -1.
                line.extend_from_slice(
                    format!(" {:2}/{:2}\n", h.users as u32, h.maxusers as u32).as_bytes(),
                );
            } else {
                line.push(b'\n');
            }
            con_print_bytes(&line);
            n += 1;
        }
        SLIST_LAST_SHOWN = n;
    }
}

/// `PrintSlistTrailer`
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_PrintSlistTrailer() {
    // SAFETY: caller contract
    unsafe {
        if c::hostCacheCount != 0 {
            con_print("== end list ==\n\n");
        } else {
            con_print("No Quake servers found.\n\n");
        }
    }
}

/// `NET_SlistSort` (the C bubble sort, byte-for-byte strcmp order)
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_SlistSort() {
    // SAFETY: caller contract
    unsafe {
        let count = c::hostCacheCount;
        if count > 1 {
            for i in 0..count {
                for j in (i + 1)..count {
                    let a = CStr::from_ptr((*host(j)).name.as_ptr());
                    let b = CStr::from_ptr((*host(i)).name.as_ptr());
                    if a.to_bytes() < b.to_bytes() {
                        core::ptr::swap(host(i), host(j));
                    }
                }
            }
        }
    }
}

static mut SLIST_PRINT_BUF: [u8; 64] = [0; 64];

/// `NET_SlistPrintServer` (returns the C-style static buffer)
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_SlistPrintServer(idx: usize) -> *const c_char {
    // SAFETY: caller contract; the static buffer mirrors the C original
    unsafe {
        if idx >= c::hostCacheCount {
            return c"".as_ptr();
        }
        let h = &*host(idx);
        let mut s;
        if h.maxusers != 0 {
            s = pad_trunc(&h.name, 17);
            s.push(b' ');
            s.extend_from_slice(&pad_trunc(&h.map, 17));
            // COMPAT: `%2u` on an int -- see rust_net_PrintSlist
            s.extend_from_slice(
                format!(" {:2}/{:2}\n", h.users as u32, h.maxusers as u32).as_bytes(),
            );
        } else {
            s = pad_trunc(&h.name, 19);
            s.push(b' ');
            s.extend_from_slice(&pad_trunc(&h.map, 19));
            s.push(b'\n');
        }
        let buf = &mut *core::ptr::addr_of_mut!(SLIST_PRINT_BUF);
        let n = s.len().min(buf.len() - 1);
        buf[..n].copy_from_slice(&s[..n]);
        buf[n] = 0;
        buf.as_ptr().cast()
    }
}

/// `NET_SlistPrintServerName`
///
/// # Safety
/// Single-threaded host frame; the returned pointer is into the hostcache.
#[no_mangle]
pub unsafe extern "C" fn rust_net_SlistPrintServerName(idx: usize) -> *const c_char {
    // SAFETY: caller contract
    unsafe {
        if idx >= c::hostCacheCount {
            return c"".as_ptr();
        }
        (*host(idx)).cname.as_ptr()
    }
}

/// `NET_Close` (sfunc.Close + pool return; no raise-capable callees)
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_Close(sock: *mut QSocket) {
    // SAFETY: caller contract
    unsafe {
        if sock.is_null() {
            return;
        }
        if (*sock).disconnected {
            return;
        }

        rust_net_SetNetTime();

        // call the driver_Close function
        let d = driver((*sock).driver);
        ((*d).close.expect("driver Close"))(sock);

        rust_net_FreeQSocket(sock);
    }
}

/// `NET_CheckNewConnections` (loop + dgrm heartbeats beneath: no raises)
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_CheckNewConnections() -> *mut QSocket {
    // SAFETY: caller contract
    unsafe {
        rust_net_SetNetTime();

        c::net_driverlevel = 0;
        while c::net_driverlevel < net_numdrivers {
            let d = driver(c::net_driverlevel);
            if (*d).initialized && ((c::net_driverlevel == 0) || c::listening) {
                let ret = ((*d)
                    .check_new_connections
                    .expect("driver CheckNewConnections"))();
                if !ret.is_null() {
                    return ret;
                }
            }
            c::net_driverlevel += 1;
        }

        core::ptr::null_mut()
    }
}

/// `NET_ListAddresses`
///
/// # Safety
/// `addresses` has `maxaddresses` slots; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_ListAddresses(
    addresses: *mut QHostAddr,
    maxaddresses: c_int,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let mut result = 0;
        c::net_driverlevel = 0;
        while c::net_driverlevel < net_numdrivers {
            let d = driver(c::net_driverlevel);
            if (*d).initialized {
                if let Some(qa) = (*d).query_addresses {
                    result += qa(addresses.add(result as usize), maxaddresses - result);
                }
            }
            c::net_driverlevel += 1;
        }
        result
    }
}

// =============================================================================
// Phase 7 M9c (T9.1): the fourteen dispatch funnels.
//
// ADR-009: every raise-capable callee below is a driver-vtable call (plus the
// cvar/command block of NET_Init), and each one goes through its own
// `NetMain_Glue_*` `Host_Guard` trampoline in a pure C frame. The five cores
// that can observe a caught jump return the status untouched; their C wrapper
// re-issues it with `Host_Reraise`.
// =============================================================================

/// `quakedef.h:214` -- `#define MAX_SCOREBOARD 16`.
const MAX_SCOREBOARD: usize = 16;

/// `enum slistScope_e { SLIST_LOOP, ... }` (net.h:108).
const SLIST_LOOP: c_int = 0;

/// `slistStartTime` (net_main.c:63) -- C file static, no external declaration.
static mut SLIST_START_TIME: f64 = 0.0;

/// `slistActiveTime` (net_main.c:64) -- C file static.
static mut SLIST_ACTIVE_TIME: f64 = 0.0;

/// `slistSendProcedure` (net_main.c:71). C's initializer leaves `arg`
/// implicitly NULL; `Slist_Send` is `rust_net_Slist_Send` here.
static mut SLIST_SEND_PROCEDURE: PollProcedure = PollProcedure {
    next: core::ptr::null_mut(),
    next_time: 0.0,
    procedure: Some(rust_net_Slist_Send),
    arg: core::ptr::null_mut(),
};

/// `slistPollProcedure` (net_main.c:72)
static mut SLIST_POLL_PROCEDURE: PollProcedure = PollProcedure {
    next: core::ptr::null_mut(),
    next_time: 0.0,
    procedure: Some(rust_net_Slist_Poll),
    arg: core::ptr::null_mut(),
};

/// `pollProcedureList` (net_main.c:1076) -- C file static. Only the list *head*
/// is Rust-owned; the nodes stay caller-owned (ADR-011), exactly as in C.
static mut POLL_PROCEDURE_LIST: *mut PollProcedure = core::ptr::null_mut();

/// `host_client->netconnection`, through the C accessor: `host_client` and
/// `svs` are server state and the net stratum builds with `use_rust_host=false`.
///
/// # Safety
/// `host_client` currently addresses a live `svs.clients` slot.
unsafe fn host_client_connection() -> *mut QSocket {
    // SAFETY: caller contract
    unsafe { c::net_main::NetMain_HostClientConnection().cast() }
}

/// `NET_Slist_f`
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_Slist_f() {
    // SAFETY: caller contract
    unsafe {
        if c::net_main::slistInProgress {
            return;
        }

        if !c::net_main::slist_silent {
            con_print("Looking for Quake servers...\n");
            rust_net_PrintSlistHeader();
        }

        c::net_main::slistInProgress = true;
        SLIST_START_TIME = c::Sys_DoubleTime();
        SLIST_ACTIVE_TIME = SLIST_START_TIME;

        rust_net_SchedulePollProcedure(&raw mut SLIST_SEND_PROCEDURE, 0.0);
        rust_net_SchedulePollProcedure(&raw mut SLIST_POLL_PROCEDURE, 0.1);

        c::hostCacheCount = 0;
    }
}

/// `Slist_Send` (net_main.c:496, `static` there). `SearchForHosts` reaches
/// `Datagram_SearchForHosts`, which the M9c raise audit cleared: no guard.
///
/// # Safety
/// Called as a `PollProcedure.procedure` callback from `NET_Poll`.
#[no_mangle]
pub unsafe extern "C" fn rust_net_Slist_Send(_unused: *mut c_void) {
    // SAFETY: caller contract
    unsafe {
        c::net_driverlevel = 0;
        while c::net_driverlevel < net_numdrivers {
            let d = driver(c::net_driverlevel);
            let skip = (c::net_dgrm_orch::slist_scope != SLIST_LOOP && c::net_driverlevel == 0)
                || !(*d).initialized;
            if !skip {
                ((*d).search_for_hosts.expect("driver SearchForHosts"))(true);
            }
            c::net_driverlevel += 1;
        }

        if (c::Sys_DoubleTime() - SLIST_START_TIME) < 0.5 {
            rust_net_SchedulePollProcedure(&raw mut SLIST_SEND_PROCEDURE, 0.75);
        }
    }
}

/// `Slist_Poll` (net_main.c:511, `static` there)
///
/// # Safety
/// Called as a `PollProcedure.procedure` callback from `NET_Poll`.
#[no_mangle]
pub unsafe extern "C" fn rust_net_Slist_Poll(_unused: *mut c_void) {
    // SAFETY: caller contract
    unsafe {
        c::net_driverlevel = 0;
        while c::net_driverlevel < net_numdrivers {
            let d = driver(c::net_driverlevel);
            let skip = (c::net_dgrm_orch::slist_scope != SLIST_LOOP && c::net_driverlevel == 0)
                || !(*d).initialized;
            if !skip && ((*d).search_for_hosts.expect("driver SearchForHosts"))(false) {
                SLIST_ACTIVE_TIME = c::Sys_DoubleTime(); // something was sent, reset the timer.
            }
            c::net_driverlevel += 1;
        }

        if !c::net_main::slist_silent {
            rust_net_PrintSlist();
        }

        if (c::Sys_DoubleTime() - SLIST_ACTIVE_TIME) < 1.5 {
            rust_net_SchedulePollProcedure(&raw mut SLIST_POLL_PROCEDURE, 0.1);
            return;
        }

        if !c::net_main::slist_silent {
            rust_net_PrintSlistTrailer();
        }
        c::net_main::slistInProgress = false;
        c::net_main::slist_silent = false;
        c::net_dgrm_orch::slist_scope = SLIST_LOOP;
    }
}

/// `NET_Connect`. Returns a `Host_Guard` status; the socket comes back in
/// `out` (ADR-009 -- `dfunc.Connect` reaches `Datagram_Connect`, which raises).
///
/// # Safety
/// `out` is a writable `qsocket_t *` slot; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_Connect(host: *const c_char, out: *mut *mut QSocket) -> c_int {
    // SAFETY: caller contract
    unsafe {
        // the `host` parameter shadows the module's hostcache helper, so the
        // helper is spelled `self::host` throughout this function
        let mut host = host;
        let mut numdrivers = net_numdrivers;

        *out = core::ptr::null_mut();

        if c::net_main::harness_netreplay {
            *out = c::net_main::Harness_NetReplayConnect().cast(); /* Phase 5 M8: replayed session */
            return HOST_GUARD_OK;
        }

        rust_net_SetNetTime();

        if !host.is_null() && *host == 0 {
            host = core::ptr::null();
        }

        'just_do_it: {
            if !host.is_null() {
                if c::net_main::q_strcasecmp(host, c"local".as_ptr()) == 0 {
                    numdrivers = 1;
                    break 'just_do_it;
                }

                if c::hostCacheCount != 0 {
                    let mut n = 0usize;
                    while n < c::hostCacheCount {
                        if c::net_main::q_strcasecmp(host, (*self::host(n)).name.as_ptr()) == 0 {
                            host = (*self::host(n)).cname.as_ptr();
                            break;
                        }
                        n += 1;
                    }
                    if n < c::hostCacheCount {
                        break 'just_do_it;
                    }
                }
            }

            c::net_main::slist_silent = !host.is_null();
            rust_net_Slist_f();

            while c::net_main::slistInProgress {
                rust_net_Poll();
            }

            if host.is_null() {
                if c::hostCacheCount != 1 {
                    return HOST_GUARD_OK; // *out is NULL
                }
                host = (*self::host(0)).cname.as_ptr();
                let mut line = b"Connecting to...\n".to_vec();
                line.extend_from_slice(&field_bytes(&(*self::host(0)).name));
                line.extend_from_slice(b" @ ");
                line.extend_from_slice(CStr::from_ptr(host).to_bytes());
                line.extend_from_slice(b"\n\n");
                con_print_bytes(&line);
            }

            if c::hostCacheCount != 0 {
                let mut n = 0usize;
                while n < c::hostCacheCount {
                    if c::net_main::q_strcasecmp(host, (*self::host(n)).name.as_ptr()) == 0 {
                        host = (*self::host(n)).cname.as_ptr();
                        break;
                    }
                    n += 1;
                }
            }
        }

        // JustDoIt:
        c::net_driverlevel = 0;
        while c::net_driverlevel < numdrivers {
            let d = driver(c::net_driverlevel);
            if (*d).initialized {
                let mut ret: *mut c_void = core::ptr::null_mut();
                let g = c::net_main::NetMain_Glue_DriverConnect(host, &mut ret);
                if g != HOST_GUARD_OK {
                    return g;
                }
                if !ret.is_null() {
                    *out = ret.cast();
                    return HOST_GUARD_OK;
                }
            }
            c::net_driverlevel += 1;
        }

        if !host.is_null() {
            con_print("\n");
            rust_net_PrintSlistHeader();
            rust_net_PrintSlist();
            rust_net_PrintSlistTrailer();
        }

        HOST_GUARD_OK // *out is NULL
    }
}

/// `NET_GetMessage`. Returns a `Host_Guard` status; the int result comes back
/// in `out` (ADR-009 -- `sfunc.QGetMessage` reaches `Datagram_GetMessage`).
///
/// # Safety
/// `out` is a writable `int` slot; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_GetMessage(sock: *mut QSocket, out: *mut c_int) -> c_int {
    // SAFETY: caller contract
    unsafe {
        if sock.is_null() {
            *out = -1;
            return HOST_GUARD_OK;
        }

        if (*sock).disconnected {
            con_print("NET_GetMessage: disconnected socket\n");
            *out = -1;
            return HOST_GUARD_OK;
        }

        if c::net_main::harness_netreplay && c::net_main::Harness_NetReplayOwns(sock.cast()) {
            *out = c::net_main::Harness_NetReplayGetMessage();
            return HOST_GUARD_OK;
        }

        rust_net_SetNetTime();

        let mut ret: c_int = 0;
        let g = c::net_main::NetMain_Glue_QGetMessage(sock.cast(), &mut ret);
        if g != HOST_GUARD_OK {
            return g;
        }

        // see if this connection has timed out
        if ret == 0 && (*sock).driver != 0 {
            // COMPAT: ADR-010. C compares `double - double > float`: the cvar's
            // f32 `value` is promoted to double by the usual arithmetic
            // conversions, so the comparison happens in f64 over an f32-rounded
            // threshold. Reproduced exactly; do not reassociate.
            if c::net_time - (*sock).last_message_time
                > f64::from(c::net_dgrm_orch::net_messagetimeout.value)
            {
                rust_net_Close(sock);
                *out = -1;
                return HOST_GUARD_OK;
            }
        }

        if ret > 0 {
            /* QGetMessage returns 1 for reliable and 2 for unreliable, which is
            exactly the capture's `kind` encoding (see harness.h) */
            let kind = ret;
            c::net_main::Harness_NetCapture(
                0,
                (*sock).driver,
                kind,
                c::net_message.data,
                c::net_message.cursize,
            );
            if (*sock).driver != 0 {
                (*sock).last_message_time = c::net_time;
                if ret == 1 {
                    c::messagesReceived += 1;
                } else if ret == 2 {
                    c::unreliableMessagesReceived += 1;
                }
            }
        }

        *out = ret;
        HOST_GUARD_OK
    }
}

/// `NET_GetServerMessage`. Returns a `Host_Guard` status; the qsocket comes
/// back in `out` (ADR-009 -- `QGetAnyMessage` reaches
/// `Datagram_GetAnyMessage`).
///
/// # Safety
/// `out` is a writable `qsocket_t *` slot; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_GetServerMessage(out: *mut *mut QSocket) -> c_int {
    // SAFETY: caller contract
    unsafe {
        *out = core::ptr::null_mut();

        c::net_driverlevel = 0;
        while c::net_driverlevel < net_numdrivers {
            let d = driver(c::net_driverlevel);
            if (*d).initialized {
                let mut s: *mut c_void = core::ptr::null_mut();
                let g = c::net_main::NetMain_Glue_QGetAnyMessage(&mut s);
                if g != HOST_GUARD_OK {
                    return g;
                }
                if !s.is_null() {
                    let s = s.cast::<QSocket>();
                    /* kind 0 = unknown: reliability is not distinguished on this path */
                    c::net_main::Harness_NetCapture(
                        0,
                        (*s).driver,
                        0,
                        c::net_message.data,
                        c::net_message.cursize,
                    );
                    *out = s;
                    return HOST_GUARD_OK;
                }
            }
            c::net_driverlevel += 1;
        }

        HOST_GUARD_OK
    }
}

/// `NET_SendMessage`. `sfunc.QSendMessage` reaches `Datagram_SendMessage`,
/// which the M9c raise audit cleared: no guard, no status channel.
///
/// # Safety
/// `data` is a live sizebuf; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_SendMessage(sock: *mut QSocket, data: *mut SizeBuf) -> c_int {
    // SAFETY: caller contract
    unsafe {
        if sock.is_null() {
            return -1;
        }

        if (*sock).disconnected {
            con_print("NET_SendMessage: disconnected socket\n");
            return -1;
        }

        if c::net_main::harness_netreplay && c::net_main::Harness_NetReplayOwns(sock.cast()) {
            return 1; /* Phase 5 M8: the replay absorbs client output */
        }

        rust_net_SetNetTime();
        let d = driver((*sock).driver);
        let r = ((*d).qsend_message.expect("driver QSendMessage"))(sock, data);
        if r == 1 {
            c::net_main::Harness_NetCapture(1, (*sock).driver, 1, (*data).data, (*data).cursize);
        }
        if r == 1 && (*sock).driver != 0 {
            c::net_dgrm_orch::messagesSent += 1;
        }

        r
    }
}

/// `NET_SendUnreliableMessage`
///
/// # Safety
/// `data` is a live sizebuf; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_SendUnreliableMessage(
    sock: *mut QSocket,
    data: *mut SizeBuf,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        if sock.is_null() {
            return -1;
        }

        if (*sock).disconnected {
            // COMPAT: the diagnostic names NET_SendMessage, not this function
            // (net_main.c:848). Console output is a compared surface, so the
            // wrong name is preserved verbatim.
            con_print("NET_SendMessage: disconnected socket\n");
            return -1;
        }

        if c::net_main::harness_netreplay && c::net_main::Harness_NetReplayOwns(sock.cast()) {
            return 1;
        }

        rust_net_SetNetTime();
        let d = driver((*sock).driver);
        let r = ((*d)
            .send_unreliable_message
            .expect("driver SendUnreliableMessage"))(sock, data);
        if r == 1 {
            c::net_main::Harness_NetCapture(1, (*sock).driver, 2, (*data).data, (*data).cursize);
        }
        if r == 1 && (*sock).driver != 0 {
            c::net_dgrm_orch::unreliableMessagesSent += 1;
        }

        r
    }
}

/// `NET_CanSendMessage`
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_CanSendMessage(sock: *mut QSocket) -> QBoolean {
    // SAFETY: caller contract
    unsafe {
        if sock.is_null() {
            return false;
        }

        if (*sock).disconnected {
            return false;
        }

        if c::net_main::harness_netreplay && c::net_main::Harness_NetReplayOwns(sock.cast()) {
            return true;
        }

        rust_net_SetNetTime();

        let d = driver((*sock).driver);
        ((*d).can_send_message.expect("driver CanSendMessage"))(sock)
    }
}

/// `NET_SendToAll`. Returns a `Host_Guard` status; the remaining count comes
/// back in `out`. The status channel exists because the loop calls
/// `NET_GetMessage`, which reaches `Datagram_GetMessage` (ADR-009).
///
/// # Safety
/// `data` is a live sizebuf and `out` a writable `int` slot; single-threaded
/// host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_SendToAll(
    data: *mut SizeBuf,
    blocktime: f64,
    out: *mut c_int,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let mut count: c_int = 0;
        // COMPAT: C sizes these `qboolean [MAX_SCOREBOARD]` but indexes them
        // with `i < svs.maxclients`. Both writers of svs.maxclients clamp it to
        // MAX_SCOREBOARD (host.c:386-389, net_main.c:334-336), so the C arrays
        // are never overrun -- the Rust bounds check below is unreachable and
        // there is no defect to reproduce.
        /* did we write the message to the client's connection	*/
        let mut msg_init = [false; MAX_SCOREBOARD];
        /* did the msg arrive its destination (canSend state).	*/
        let mut msg_sent = [false; MAX_SCOREBOARD];

        let maxclients = c::NetMain_MaxClients();

        // C's `for (i = 0, host_client = svs.clients; ...; i++, host_client++)`:
        // host_client tracks i and is left one past the last slot on exit.
        let mut i: c_int = 0;
        while i < maxclients {
            c::net_main::NetMain_SetHostClient(i);
            /*
            if (!host_client->netconnection)
                continue;
            if (host_client->active)
            */
            let conn = host_client_connection();
            if !conn.is_null() && c::net_main::NetMain_HostClientActive() {
                if (*conn).driver == 0 {
                    rust_net_SendMessage(conn, data);
                    msg_init[i as usize] = true;
                    msg_sent[i as usize] = true;
                } else {
                    count += 1;
                    msg_init[i as usize] = false;
                    msg_sent[i as usize] = false;
                }
            } else {
                msg_init[i as usize] = true;
                msg_sent[i as usize] = true;
            }
            i += 1;
        }
        c::net_main::NetMain_SetHostClient(maxclients);

        let start = c::Sys_DoubleTime();
        while count != 0 {
            count = 0;
            let mut i: c_int = 0;
            while i < maxclients {
                c::net_main::NetMain_SetHostClient(i);
                'iteration: {
                    if !msg_init[i as usize] {
                        if rust_net_CanSendMessage(host_client_connection()) {
                            msg_init[i as usize] = true;
                            rust_net_SendMessage(host_client_connection(), data);
                        } else {
                            let mut ignored: c_int = 0;
                            let g = rust_net_GetMessage(host_client_connection(), &mut ignored);
                            if g != HOST_GUARD_OK {
                                return g;
                            }
                        }
                        count += 1;
                        break 'iteration;
                    }

                    if !msg_sent[i as usize] {
                        if rust_net_CanSendMessage(host_client_connection()) {
                            msg_sent[i as usize] = true;
                        } else {
                            let mut ignored: c_int = 0;
                            let g = rust_net_GetMessage(host_client_connection(), &mut ignored);
                            if g != HOST_GUARD_OK {
                                return g;
                            }
                        }
                        count += 1;
                    }
                }
                i += 1;
            }
            c::net_main::NetMain_SetHostClient(maxclients);
            if (c::Sys_DoubleTime() - start) > blocktime {
                break;
            }
        }

        *out = count;
        HOST_GUARD_OK
    }
}

//=============================================================================

/// `NET_Init`. Returns a `Host_Guard` status: the driver `Init` slots reach
/// `Datagram_Init`, and the cvar/command block can raise under
/// `-Duse_rust_cvar` (ADR-009).
///
/// # Safety
/// Called once, from `Host_Init`.
#[no_mangle]
pub unsafe extern "C" fn rust_net_Init() -> c_int {
    // SAFETY: caller contract
    unsafe {
        let mut i = c::COM_CheckParm(c"-port".as_ptr());
        if i == 0 {
            i = c::COM_CheckParm(c"-udpport".as_ptr());
        }

        if i != 0 {
            if i < c::com_argc - 1 {
                let arg = *c::com_argv.add((i + 1) as usize);
                c::DEFAULTnet_hostport = c_atoi(CStr::from_ptr(arg).to_bytes());
            } else {
                c::Sys_Error(c"NET_Init: you must specify a number after -port".as_ptr());
            }
        }
        c::net_hostport = c::DEFAULTnet_hostport;

        c::net_main::net_numsockets = c::NetMain_MaxClientsLimit();
        if !c::net_main::NetMain_ClsDedicated() {
            c::net_main::net_numsockets += 1;
        }
        if c::COM_CheckParm(c"-listen".as_ptr()) != 0 || c::net_main::NetMain_ClsDedicated() {
            c::listening = true;
        }

        rust_net_SetNetTime();

        for _ in 0..c::net_main::net_numsockets {
            let s = c::Mem_Alloc(core::mem::size_of::<QSocket>()).cast::<QSocket>();
            (*s).next = c::net_freeSockets.cast();
            c::net_freeSockets = s.cast();
            (*s).disconnected = true;
        }

        // allocate space for network message buffer
        c::cvar_cmd::SZ_Alloc(&raw mut c::net_message, NET_MAXMESSAGE as c_int);

        let g = c::net_main::NetMain_Glue_RegisterNetVars();
        if g != HOST_GUARD_OK {
            return g;
        }

        // initialize all the drivers
        let mut i: c_int = 0;
        c::net_driverlevel = 0;
        while c::net_driverlevel < net_numdrivers {
            let mut r: c_int = 0;
            let g = c::net_main::NetMain_Glue_DriverInit(&mut r);
            if g != HOST_GUARD_OK {
                return g;
            }
            if r != -1 {
                i += 1;
                let d = driver(c::net_driverlevel);
                (*d).initialized = true;
                if c::listening {
                    ((*d).listen.expect("driver Listen"))(true);
                }
            }
            c::net_driverlevel += 1;
        }

        /* Loop_Init() returns -1 for dedicated server case,
         * therefore the i == 0 check is correct */
        if i == 0 && c::net_main::NetMain_ClsDedicated() {
            c::Sys_Error(c"Network not available!".as_ptr());
        }

        let ip4 = (&raw const c::my_ipv4_address).cast::<c_char>();
        if *ip4 != 0 {
            c::Con_DPrintf(c"IPv4 address %s\n".as_ptr(), ip4);
        }
        let ip6 = (&raw const c::my_ipv6_address).cast::<c_char>();
        if *ip6 != 0 {
            c::Con_DPrintf(c"IPv6 address %s\n".as_ptr(), ip6);
        }

        HOST_GUARD_OK
    }
}

/// `NET_Shutdown`. `Close` and `Shutdown` are two of the vtable slots the M9c
/// raise audit cleared, so no status channel.
///
/// # Safety
/// Called once, from `Host_Shutdown`.
#[no_mangle]
pub unsafe extern "C" fn rust_net_Shutdown() {
    // SAFETY: caller contract
    unsafe {
        rust_net_SetNetTime();

        // COMPAT: NET_Close -> NET_FreeQSocket relinks `sock` onto the free
        // list before `sock->next` is read here, so this walk continues into
        // the free list rather than through the rest of the active list
        // (net_main.c:1060-1061). Preserved verbatim.
        let mut sock = c::net_activeSockets.cast::<QSocket>();
        while !sock.is_null() {
            rust_net_Close(sock);
            sock = (*sock).next.cast();
        }

        //
        // shutdown the drivers
        //
        c::net_driverlevel = 0;
        while c::net_driverlevel < net_numdrivers {
            let d = driver(c::net_driverlevel);
            if (*d).initialized {
                ((*d).shutdown.expect("driver Shutdown"))();
                (*d).initialized = false;
            }
            c::net_driverlevel += 1;
        }
    }
}

/// `NET_Poll`
///
/// ADR-009: the reachable `PollProcedure.procedure` set is `Slist_Send`,
/// `Slist_Poll`, `quake_rs_dgrm_test_poll` and `quake_rs_dgrm_test2_poll`;
/// none of the four can raise, so this indirect call needs no `Host_Guard`
/// and the funnel needs no status channel.
///
/// # Safety
/// Single-threaded host frame; the list holds caller-owned nodes.
#[no_mangle]
pub unsafe extern "C" fn rust_net_Poll() {
    // SAFETY: caller contract
    unsafe {
        rust_net_SetNetTime();

        // COMPAT: `pp->next` is re-read after the callback ran, and a callback
        // that re-schedules `pp` (Slist_Send and Slist_Poll both do) has
        // already rewritten it -- so the walk resumes from the *new* successor,
        // not the one that was there on entry (net_main.c:1084-1090).
        let mut pp = POLL_PROCEDURE_LIST;
        while !pp.is_null() {
            if (*pp).next_time > c::net_time {
                break;
            }
            POLL_PROCEDURE_LIST = (*pp).next;
            ((*pp).procedure.expect("poll procedure"))((*pp).arg);
            pp = (*pp).next;
        }
    }
}

/// `SchedulePollProcedure`
///
/// # Safety
/// `proc_` is a live `PollProcedure` the caller keeps alive for as long as it
/// stays scheduled; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_net_SchedulePollProcedure(
    proc_: *mut PollProcedure,
    time_offset: f64,
) {
    // SAFETY: caller contract
    unsafe {
        (*proc_).next_time = c::Sys_DoubleTime() + time_offset;

        let mut pp = POLL_PROCEDURE_LIST;
        let mut prev: *mut PollProcedure = core::ptr::null_mut();
        while !pp.is_null() {
            if (*pp).next_time >= (*proc_).next_time {
                break;
            }
            prev = pp;
            pp = (*pp).next;
        }

        if prev.is_null() {
            (*proc_).next = POLL_PROCEDURE_LIST;
            POLL_PROCEDURE_LIST = proc_;
            return;
        }

        (*proc_).next = pp;
        (*prev).next = proc_;
    }
}
