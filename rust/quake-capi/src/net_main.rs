//! Phase 5 M9: the ADR-009-safe core of net_main.c -- qsocket pool
//! management, SetNetTime, the qsocket accessors, the slist UI helpers,
//! the listen/maxplayers/port command handlers, and the leaf driver loops
//! (Close, CheckNewConnections, ListAddresses). net_main.c keeps trampolines
//! under `USE_RUST_NET` (the Phase 3 in-file idiom).
//!
//! Deliberately NOT ported (the M9 ADR-009 audit, recorded in the task
//! plan): the dispatch funnels NET_Connect / NET_GetMessage /
//! NET_GetServerMessage / NET_Send* / NET_SendToAll / NET_Poll /
//! SchedulePollProcedure and NET_Init/Shutdown. `Host_Error`-capable code
//! runs beneath them (the dgrm glue's re-raise, `_Datagram_ServerControl
//! Packet` -> SV_ConnectClient, the MSG-writer glue under SearchForHosts),
//! and a longjmp must never unwind a Rust frame -- those functions ARE the
//! required C frames until Phase 7 statusizes the layers below. The paths
//! ported here have only `Sys_Error` exits (no longjmp) beneath them.
//!
//! Ownership (ADR-007): pool heads, hostcache, counters, slist state and
//! net_time stay C-owned in net_main.c; `slistLastShown` (touched only by
//! the ported print helpers) moves to Rust module state.

use core::ffi::{c_char, c_int, CStr};

use quake_c_sys as c;
use quake_types::net::{HostCache, NetDriver, QHostAddr, QSocket};

extern "C" {
    /// net_bsd.c / net_win.c (ADR-011 mirror; indexed like C's arrays)
    static mut net_drivers: NetDriver;
    static net_numdrivers: c_int;
    static mut hostcache: HostCache;
}

fn driver(idx: c_int) -> *mut NetDriver {
    // SAFETY: idx < net_numdrivers, matching the C array
    unsafe { (&raw mut net_drivers).add(idx as usize) }
}

fn host(idx: usize) -> *mut HostCache {
    // SAFETY: idx < HOSTCACHESIZE, matching the C array
    unsafe { (&raw mut hostcache).add(idx) }
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

/// C `atoi` (see quake_net::udp)
fn c_atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || (0x09..=0x0d).contains(&s[i])) {
        i += 1;
    }
    let mut sign = 1i64;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        if s[i] == b'-' {
            sign = -1;
        }
        i += 1;
    }
    let mut v: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = (v * 10 + (s[i] - b'0') as i64).clamp(i64::MIN / 2, i64::MAX / 2);
        i += 1;
    }
    (sign * v).clamp(i32::MIN as i64, i32::MAX as i64) as i32
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
                line.extend_from_slice(format!(" {:2}/{:2}\n", h.users, h.maxusers).as_bytes());
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
            s.extend_from_slice(format!(" {:2}/{:2}\n", h.users, h.maxusers).as_bytes());
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
