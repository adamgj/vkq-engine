//! `Quake/net_dgrm.c`'s orchestration half -- Phase 7 M9b / T9.2.
//!
//! Pattern A whole-file swap. Under `-Duse_rust_net` the C file leaves the
//! Meson `srcs` list and `Quake/net_dgrm_orch_glue.c` supplies the C-visible
//! cvars, the driver-table wrappers and the ADR-009 raise trampolines; the
//! bodies live here. The Phase 5 `Quake/net_dgrm_glue.c` is untouched -- it
//! owns `packetBuffer`, the six packet counters and the eight wire slots, and
//! this module must not redefine any of them.
//!
//! Ported: the whole file except the reliable-wire half already in Phase 5's
//! `net_dgrm.rs`. `NET_Ban_f` (`net_dgrm.c:81-133`) is *not* ported -- its
//! `BAN_TEST` gate is commented out at `:23` and defined by no build file, so
//! it is preprocessed away in every configuration.
//!
//! ADR-009: the driver-table slots have no status channel of their own
//! (`net_defs.h`'s `qsocket_t *(*QGetAnyMessage)(void)` and friends), so the
//! **C wrapper is the raise frame**: a Rust core returns a `Host_Guard`
//! status, the wrapper in `net_dgrm_orch_glue.c` calls `Host_Reraise` and then
//! returns the out-parameter. No `longjmp` crosses a Rust frame.
//!
//! The four live raise sites are all in `_Datagram_ServerControlPacket`
//! (`Cmd_ExecuteString`, `SV_DropClient`, `SV_ConnectClient`) plus the three
//! `SCR_UpdateScreen` calls in the connect handshake; each goes through a
//! `NetDgrmOrch_Glue_*` trampoline holding an argument struct. `:1166-1559`
//! (server discovery) and `:305-565` (`test`/`test2`) contain none, which is
//! why their entry points carry no status channel.
//!
//! COMPAT: transliterated bug-for-bug. The preserved defects are documented at
//! their sites; see the module's `// COMPAT:` comments.

use core::ffi::{c_char, c_int, c_uint, c_void, CStr};

use quake_c_sys as c;
use quake_c_sys::host::HOST_GUARD_OK;
use quake_c_sys::net_dgrm_orch::*;
use quake_net::cnum::c_atoi;
use quake_net::msg;
use quake_net::msg::MsgReader;
use quake_net::sizebuf::{SizeBuf, WireError};
use quake_types::host::{Server, ServerStatic, NUM_PING_TIMES};
use quake_types::net::{
    HostCache, NetLanDriver, PollProcedure, QBoolean, QHostAddr, QSockAddr, QSocket, SysSocket,
    CCREP_ACCEPT, CCREP_PLAYER_INFO, CCREP_RCON, CCREP_REJECT, CCREP_RULE_INFO, CCREP_SERVER_INFO,
    CCREQ_CONNECT, CCREQ_PLAYER_INFO, CCREQ_RCON, CCREQ_RULE_INFO, CCREQ_SERVER_INFO,
    HOSTCACHESIZE, NETFLAG_CTL, NETFLAG_LENGTH_MASK, NET_DATAGRAMSIZE, NET_NAMELEN,
    NET_PROTOCOL_VERSION,
};

// ############################ agent A: lifecycle, stats, rcon flush ##########

// -----------------------------------------------------------------------------
// module state (ADR-007: the host frame is single-threaded, so `static mut`
// mirrors the C storage class one-for-one, matching net_main.rs / net.rs)
// -----------------------------------------------------------------------------

/// `static int net_landriverlevel` (net_dgrm.c:36). `dfunc` (`:34`) is
/// `net_landrivers[net_landriverlevel]`; every use below expands through
/// [`landriver`]. Shared with agents B, C1, C2 and D -- one definition only.
pub(crate) static mut NET_LANDRIVERLEVEL: c_int = 0;

/// `static int myDriverLevel` (net_dgrm.c:59). Written by `Datagram_Init`,
/// read by agent D at `:386` and `:520`.
pub(crate) static mut MY_DRIVER_LEVEL: c_int = 0;

/// `static double heartbeat_time` (net_dgrm.c:63) -- when to send the next
/// master-server heartbeat. Reset by `Datagram_Listen`, read/written by
/// agent B at `:1129`/`:1136`.
pub(crate) static mut HEARTBEAT_TIME: f64 = 0.0;

/// `StrAddr`'s function-static `char buf[34]` (net_dgrm.c:66).
///
/// COMPAT: 34 is one byte more than needed -- 16 bytes x 2 hex digits = 32
/// plus the NUL is 33, and `buf[33]` is never written. Size kept as the C
/// declared it.
static mut STR_ADDR_BUF: [c_char; 34] = [0; 34];

/// `Strip_Port`'s function-static `char noport[MAX_QPATH]` (net_dgrm.c:282).
/// `MAX_QPATH` is 64 (q_types.h:240).
static mut STRIP_PORT_NOPORT: [c_char; MAX_QPATH] = [0; MAX_QPATH];

/// `static struct qsockaddr rcon_response_address` (net_dgrm.c:668). Agent B
/// sets all three at `:927-929` before `Con_Redirect (Datagram_Rcon_Flush)`.
pub(crate) static mut RCON_RESPONSE_ADDRESS: QSockAddr = QSockAddr::zeroed();

/// `static sys_socket_t rcon_response_socket` (net_dgrm.c:669)
pub(crate) static mut RCON_RESPONSE_SOCKET: SysSocket = 0;

/// `static sys_socket_t rcon_response_landriver` (net_dgrm.c:670).
///
/// COMPAT: the C types this as `sys_socket_t`, but `:929` stores
/// `net_landriverlevel` in it -- an *index* into `net_landrivers[]`, not a
/// socket handle. On Windows `sys_socket_t` is `UINT_PTR`. The wrong type is
/// preserved; the one use site (`:687`) casts.
pub(crate) static mut RCON_RESPONSE_LANDRIVER: SysSocket = 0;

/// `q_types.h:240`
const MAX_QPATH: usize = 64;

/// `net_sys.h:63` / `:101` (unix, amiga) and the Winsock `INVALID_SOCKET`
/// (`(SOCKET)(~0)`, i.e. all-ones `UINT_PTR`). No such constant existed
/// anywhere in `rust/` before M9b; agents B, C1, C2 and D need it too, so it
/// is a hoisting candidate.
#[cfg(not(windows))]
const INVALID_SOCKET: SysSocket = -1;
#[cfg(windows)]
const INVALID_SOCKET: SysSocket = usize::MAX;

/// `net_landrivers[idx]`, i.e. the `dfunc`/`sfunc` macros. Same reasoning as
/// `net_dgrm.rs`'s private `landriver`: the C array is an incomplete type, so
/// a Rust extern could only describe its first element and `.add(idx)` would
/// be arithmetic outside the declared object. `NetMain_LanDrivers()` hands
/// back the real base pointer, giving the offset provenance over the whole
/// array (ADR-004).
fn landriver(idx: c_int) -> *mut NetLanDriver {
    // SAFETY: every call site bounds idx by net_numlandrivers, the count that
    // sized the array NetMain_LanDrivers returns. The one exception is
    // rcon_flush's `RCON_RESPONSE_LANDRIVER`, which the C does not bound
    // either (net_dgrm.c:687) -- see the note there.
    unsafe {
        c::NetMain_LanDrivers()
            .cast::<NetLanDriver>()
            .add(idx as usize)
    }
}

/// `Con_Printf ("%s", <bytes>)`, as in net_main.rs
fn con_print_bytes(text: &[u8]) {
    let mut b = text.to_vec();
    b.push(0);
    // SAFETY: b is NUL-terminated
    unsafe {
        c::Con_Printf(c"%s".as_ptr(), b.as_ptr());
    }
}

/// `net_dgrm.c:34` -- `#define dfunc net_landrivers[net_landriverlevel]`.
fn dfunc() -> *mut NetLanDriver {
    // SAFETY: net_landriverlevel is only ever walked by the bounded loops in
    // this file, exactly as the C macro assumes.
    landriver(unsafe { NET_LANDRIVERLEVEL })
}

/// `&net_message`.
fn net_message() -> *mut c::sizebuf_t {
    &raw mut c::net_message
}

/// `Con_SafePrintf ("%s", <str>)`. The `%s` indirection keeps a `%` in the
/// text from being read as a specifier, as everywhere else in this file.
fn con_safe_print(text: &str) {
    con_safe_print_bytes(text.as_bytes());
}

/// `Con_SafePrintf ("%s", <bytes>)`.
fn con_safe_print_bytes(text: &[u8]) {
    let mut b = text.to_vec();
    b.push(0);
    // SAFETY: b is NUL-terminated
    unsafe {
        c::Con_SafePrintf(c"%s".as_ptr(), b.as_ptr());
    }
}

/// `Con_DWarning ("%s", <str>)`.
fn con_dwarning(text: &str) {
    let mut b = text.as_bytes().to_vec();
    b.push(0);
    // SAFETY: b is NUL-terminated
    unsafe {
        c::Con_DWarning(c"%s".as_ptr(), b.as_ptr());
    }
}

/// `dfunc.AddrToString (addr)` as owned bytes. The landriver returns a
/// pointer into its own static, so the bytes are copied before the next call
/// can clobber them.
///
/// # Safety
/// `ld` is a live landriver with its `AddrToString` slot installed; `addr`
/// points at a live `struct qsockaddr`.
unsafe fn addr_to_string(ld: *mut NetLanDriver, addr: *mut QSockAddr) -> Vec<u8> {
    // SAFETY: caller contract; AddrToString always returns a NUL-terminated
    // pointer (net_bsd.c/net_win.c both return a function-static buffer).
    unsafe {
        let p = ((*ld).addr_to_string.expect("landriver AddrToString"))(addr, false);
        CStr::from_ptr(p).to_bytes().to_vec()
    }
}

/// `net_dgrm.c:54` -- `#define MAX_MASTERS 8`, the extent of
/// `cvar_t net_masters[MAX_MASTERS]`. Pinned by the `[cvar_t; 8]` mirror in
/// `quake-c-sys/src/net_dgrm_orch.rs`.
const NET_MASTERS_COUNT: usize = 8;

/// `keys.h:141` -- `key_menu`, the fourth `keydest_t` enumerator
/// (`key_game`, `key_console`, `key_message`, `key_menu`).
const KEY_MENU: c_int = 3;

// -----------------------------------------------------------------------------
// net_dgrm.c:65 -- StrAddr
// -----------------------------------------------------------------------------

/// `static char *StrAddr (struct qsockaddr *addr)` (net_dgrm.c:65).
///
/// Returns a pointer into [`STR_ADDR_BUF`], exactly as the C returned its
/// function-static `buf`: the previous result is clobbered by the next call.
/// Deliberately not an owned `String` -- the aliasing is C's behaviour and
/// callers (agent C2, `:1630`/`:1631`) hand it straight to a `%s`.
///
/// # Safety
/// Single-threaded host frame; `addr` points at a live `struct qsockaddr`.
pub(crate) unsafe fn str_addr(addr: *const QSockAddr) -> *const c_char {
    // SAFETY: caller contract; QSockAddr is 64 bytes (pinned by the const
    // assert in quake-types/src/net.rs), so the 16 bytes read are in range,
    // and the two writes per iteration stay within STR_ADDR_BUF's 34
    unsafe {
        const HEX: [u8; 16] = *b"0123456789abcdef";
        let src = addr.cast::<u8>();
        let dst = (&raw mut STR_ADDR_BUF).cast::<c_char>();
        // C: for (n = 0; n < 16; n++) q_snprintf (buf + n*2, ..., "%02x", *p++)
        for n in 0..16usize {
            let b = *src.add(n);
            *dst.add(n * 2) = HEX[(b >> 4) as usize] as c_char;
            *dst.add(n * 2 + 1) = HEX[(b & 0x0f) as usize] as c_char;
        }
        // q_snprintf's terminator for the last field
        *dst.add(32) = 0;
        dst.cast_const()
    }
}

// -----------------------------------------------------------------------------
// net_dgrm.c:138 -- Datagram_GetAnyMessage
// -----------------------------------------------------------------------------

/// `qsocket_t *Datagram_GetAnyMessage (void)` (net_dgrm.c:138), as an
/// ADR-009 status core. The vtable slot has no status channel
/// (`qsocket_t *(*QGetAnyMessage)(void)`), so `net_dgrm_orch_glue.c`'s
/// `Datagram_GetAnyMessage` is the raise frame and re-issues the jump.
///
/// # Safety
/// Single-threaded host frame; `out` points at one writable `qsocket_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_get_any_message(out: *mut *mut QSocket) -> c_int {
    // SAFETY: caller contract; all engine globals are host-thread-only
    unsafe {
        *out = core::ptr::null_mut();

        // C declares `struct qsockaddr addr;` uninitialised and lets
        // dfunc.Read fill it; zeroing is not observable (every read of it is
        // preceded by a successful Read).
        let mut addr = QSockAddr::zeroed();

        NET_LANDRIVERLEVEL = 0;
        while NET_LANDRIVERLEVEL < net_numlandrivers {
            let ld = landriver(NET_LANDRIVERLEVEL);
            if !(*ld).initialized {
                NET_LANDRIVERLEVEL += 1;
                continue;
            }
            let sock = (*ld).listening_sock;
            if sock == INVALID_SOCKET {
                NET_LANDRIVERLEVEL += 1;
                continue;
            }

            loop {
                let length = ((*ld).read.expect("landriver Read"))(
                    sock,
                    (&raw mut c::packetBuffer).cast::<u8>(),
                    NET_DATAGRAMSIZE as c_int,
                    &mut addr,
                );
                if length == -1 || length == 0 {
                    // no more packets, move on to the next.
                    break;
                }

                // COMPAT: runt packets are dropped here WITHOUT bumping
                // shortPacketCount -- the rel layer's counter is untouched
                // on this path (net_dgrm.c:159).
                if length < 4 {
                    continue;
                }

                // BigLong (packetBuffer.length): the field holds the raw
                // first four wire bytes, so from_be is BigLong on both
                // endiannesses.
                let header = u32::from_be(c::packetBuffer.length);
                if header & NETFLAG_CTL != 0 {
                    let st = server_control_packet(
                        sock,
                        &mut addr,
                        (&raw mut c::packetBuffer).cast::<u8>(),
                        length as c_uint,
                    );
                    // C longjmp'd out of the whole function here
                    if st != 0 {
                        return st;
                    }
                    continue;
                }

                // figure out which qsocket it was for
                let mut s = c::net_activeSockets.cast::<QSocket>();
                while !s.is_null() {
                    if (*s).driver != c::net_driverlevel || (*s).disconnected || !(*s).isvirtual {
                        s = (*s).next;
                        continue;
                    }
                    if ((*ld).addr_compare.expect("landriver AddrCompare"))(
                        &mut addr,
                        &raw mut (*s).addr,
                    ) == 0
                    {
                        // okay, looks like this is us. try to process it, and
                        // if there's new data
                        if crate::net_dgrm::rust_dgrm_ProcessPacket(length as c_uint, s) {
                            (*s).last_message_time = c::net_time;
                            // the server needs to parse that packet.
                            *out = s;
                            return 0;
                        }
                    }
                    s = (*s).next;
                }
                // stray packet... ignore it and just try the next
            }

            NET_LANDRIVERLEVEL += 1;
        }
        // COMPAT: the C `for` leaves net_landriverlevel == net_numlandrivers
        // once the scan finishes, i.e. `dfunc` is out of range until the next
        // writer resets it. Preserved.

        let mut s = c::net_activeSockets.cast::<QSocket>();
        while !s.is_null() {
            if (*s).driver != c::net_driverlevel || !(*s).isvirtual {
                s = (*s).next;
                continue;
            }

            if (*s).send_next {
                crate::net_dgrm::rust_dgrm_SendMessageNext(s);
            }
            if !(*s).can_send && (c::net_time - (*s).last_send_time) > 1.0 {
                crate::net_dgrm::rust_dgrm_ReSendMessage(s);
            }

            // net_dgrm.c:204. `.value` is a C float widened to double for the
            // comparison, exactly as the C did (ADR-010: no reassociation).
            let timeout = if (*s).ack_sequence == 0 {
                net_connecttimeout.value
            } else {
                net_messagetimeout.value
            };
            if c::net_time - (*s).last_message_time > timeout as f64 {
                // timed out, kick them
                // FIXME: add a proper challenge rather than assuming spoofers
                // won't fake acks
                //
                // ADR-009: the svs.clients scan, the host_client store and
                // SV_DropClient (false) (net_dgrm.c:208-216) all live inside
                // one Host_Guard in the glue -- SV_DropClient longjmps via
                // ClientDisconnect QC.
                let st = NetDgrmOrch_Glue_DropClient(s.cast(), false);
                if st != 0 {
                    return st;
                }
            }

            // COMPAT: `s = s->next` is read AFTER SV_DropClient, which can
            // NET_FreeQSocket this very socket and relink it onto the free
            // list. The C walks the mutated `next` too (net_dgrm.c:191).
            s = (*s).next;
        }

        0
    }
}

// -----------------------------------------------------------------------------
// net_dgrm.c:223 -- PrintStats
// -----------------------------------------------------------------------------

/// `static void PrintStats (qsocket_t *s)` (net_dgrm.c:223).
///
/// COMPAT: `%4u` is applied to `s->canSend`, a `qboolean` (C11 `_Bool`).
/// Default argument promotion makes it `int` 0/1 and printf reads it as
/// `unsigned`, so the text is `   0` / `   1` -- rendered here as a
/// width-4 right-aligned integer, not as a bool.
///
/// COMPAT: `sendSeq` has no `\n` and `recvSeq` does, so the two share one
/// line; then a bare `\n` ends the block with a blank line. The trailing
/// three-space runs are in the C format strings. Byte-preserved.
///
/// # Safety
/// Single-threaded host frame; `s` is a live qsocket.
unsafe fn print_stats(s: *mut QSocket) {
    // SAFETY: caller contract
    unsafe {
        let can_send: u32 = if (*s).can_send { 1 } else { 0 };
        con_print_bytes(format!("canSend = {can_send:>4}   \n").as_bytes());
        con_print_bytes(format!("sendSeq = {:>4}   ", (*s).send_sequence).as_bytes());
        con_print_bytes(format!("recvSeq = {:>4}   \n", (*s).receive_sequence).as_bytes());
        con_print_bytes(b"\n");
    }
}

// -----------------------------------------------------------------------------
// net_dgrm.c:231 -- NET_Stats_f
// -----------------------------------------------------------------------------

/// `static void NET_Stats_f (void)` (net_dgrm.c:231). Registered as the
/// `net_stats` xcommand; `net_dgrm_orch_glue.c` owns the `NET_Stats_f`
/// C symbol that `Cmd_AddCommand` receives.
///
/// Nothing beneath this can `Host_Error`/`Host_EndGame` (Con_Printf,
/// Cmd_Argc/Cmd_Argv, q_strcasecmp and the qsocket pool walk are all
/// non-raising), so there is no status channel.
///
/// # Safety
/// Single-threaded host frame, called from the command dispatcher.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_net_stats_f() {
    // SAFETY: caller contract
    unsafe {
        if c::Cmd_Argc() == 1 {
            // Each counter is read into a local first: they are `static
            // mut`, and `format!` would otherwise take a shared reference.
            let unreliable_sent = unreliableMessagesSent;
            let unreliable_recv = c::unreliableMessagesReceived;
            let reliable_sent = messagesSent;
            let reliable_recv = c::messagesReceived;
            let packets_sent = c::packetsSent;
            let packets_resent = c::packetsReSent;
            let packets_received = c::packetsReceived;
            let duplicate_count = c::receivedDuplicateCount;
            let short_packets = c::shortPacketCount;
            let dropped = c::droppedDatagrams;
            // byte-identical to net_dgrm.c:236-245, column alignment included
            con_print_bytes(format!("unreliable messages sent   = {unreliable_sent}\n").as_bytes());
            con_print_bytes(format!("unreliable messages recv   = {unreliable_recv}\n").as_bytes());
            con_print_bytes(format!("reliable messages sent     = {reliable_sent}\n").as_bytes());
            con_print_bytes(format!("reliable messages received = {reliable_recv}\n").as_bytes());
            con_print_bytes(format!("packetsSent                = {packets_sent}\n").as_bytes());
            con_print_bytes(format!("packetsReSent              = {packets_resent}\n").as_bytes());
            con_print_bytes(
                format!("packetsReceived            = {packets_received}\n").as_bytes(),
            );
            con_print_bytes(format!("receivedDuplicateCount     = {duplicate_count}\n").as_bytes());
            con_print_bytes(format!("shortPacketCount           = {short_packets}\n").as_bytes());
            con_print_bytes(format!("droppedDatagrams           = {dropped}\n").as_bytes());
            return;
        }

        let argv1 = c::Cmd_Argv(1);
        // C: strcmp (Cmd_Argv (1), "*") == 0
        if core::ffi::CStr::from_ptr(argv1).to_bytes() == b"*" {
            let mut s = c::net_activeSockets.cast::<QSocket>();
            while !s.is_null() {
                print_stats(s);
                s = (*s).next;
            }
            let mut s = c::net_freeSockets.cast::<QSocket>();
            while !s.is_null() {
                print_stats(s);
                s = (*s).next;
            }
            return;
        }

        let mut s = c::net_activeSockets.cast::<QSocket>();
        while !s.is_null() {
            if q_strcasecmp(argv1, (&raw const (*s).trueaddress).cast::<c_char>()) == 0
                || q_strcasecmp(argv1, (&raw const (*s).maskedaddress).cast::<c_char>()) == 0
            {
                break;
            }
            s = (*s).next;
        }

        if s.is_null() {
            s = c::net_freeSockets.cast::<QSocket>();
            while !s.is_null() {
                if q_strcasecmp(argv1, (&raw const (*s).trueaddress).cast::<c_char>()) == 0
                    || q_strcasecmp(argv1, (&raw const (*s).maskedaddress).cast::<c_char>()) == 0
                {
                    break;
                }
                s = (*s).next;
            }
        }

        if s.is_null() {
            return;
        }

        print_stats(s);
    }
}

// -----------------------------------------------------------------------------
// net_dgrm.c:280 -- Strip_Port
// -----------------------------------------------------------------------------

/// `static const char *Strip_Port (const char *host)` (net_dgrm.c:280) --
/// "recognize ip:port (based on ProQuake)".
///
/// Returns EITHER `host` unchanged (the three early-outs) OR a pointer into
/// [`STRIP_PORT_NOPORT`], with the same clobber-on-next-call lifetime the C
/// static had. Callers: agent D (`:378`, `:512`) and agent C2 (`:1825`).
///
/// # Safety
/// Single-threaded host frame; `host` is NUL-terminated or null.
pub(crate) unsafe fn strip_port(host: *const c_char) -> *const c_char {
    // SAFETY: caller contract
    unsafe {
        if host.is_null() || *host == 0 {
            return host;
        }

        // q_strlcpy (noport, host, sizeof (noport)) -- strlcpy.c:30-51 copies
        // at most siz-1 = 63 bytes and always terminates.
        //
        // COMPAT: a host longer than 63 bytes is silently truncated, and the
        // truncation is what gets returned and connected to.
        //
        // Built in a local first so no reference to the `static mut` is ever
        // created; only the bytes the C actually wrote are copied out.
        let src = host.cast::<u8>();
        let mut buf = [0u8; MAX_QPATH];
        let mut term = MAX_QPATH - 1;
        for (i, slot) in buf.iter_mut().enumerate().take(MAX_QPATH - 1) {
            let ch = *src.add(i);
            *slot = ch;
            if ch == 0 {
                term = i;
                break;
            }
        }
        buf[term] = 0;

        // strrchr (noport, ':')
        let colon = match buf[..term].iter().rposition(|&ch| ch == b':') {
            Some(i) => i,
            None => return host,
        };
        // strchr (p, ']') -- [::] should not be considered port 0
        if buf[colon..term].contains(&b']') {
            return host;
        }

        // *p++ = '\0'
        buf[colon] = 0;
        // port = atoi (p): quake_net::cnum::c_atoi is the repo's bit-exact atoi
        let port = c_atoi(&buf[colon + 1..term]);

        // COMPAT: re-specifying the port already in use prints nothing but
        // still returns the stripped copy.
        if port > 0 && port < 65536 && port != c::net_hostport {
            c::net_hostport = port;
            let hostport = c::net_hostport;
            con_print_bytes(format!("Port set to {hostport}\n").as_bytes());
        }

        let dst = (&raw mut STRIP_PORT_NOPORT).cast::<u8>();
        core::ptr::copy_nonoverlapping(buf.as_ptr(), dst, term + 1);
        dst.cast::<c_char>().cast_const()
    }
}

// -----------------------------------------------------------------------------
// net_dgrm.c:567 -- Datagram_Init
// -----------------------------------------------------------------------------

/// `int Datagram_Init (void)` (net_dgrm.c:567), as an ADR-009 status core.
///
/// Contrary to the M9b contract's storage-split note, this function registers
/// **no cvars** -- it has three `Cmd_AddCommand` calls and zero
/// `Cvar_RegisterVariable`. `sv_reportheartbeats`, `sv_public`,
/// `com_protocolname`, `net_masters[]` and `rcon_password` are never
/// registered anywhere in the tree; see m9b_a_notes.md.
///
/// The `#ifdef BAN_TEST` prologue (`:572-575`) is absent: BAN_TEST is
/// defined by nothing.
///
/// # Safety
/// Single-threaded host frame; `out` points at one writable `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_init(out: *mut c_int) -> c_int {
    // SAFETY: caller contract
    unsafe {
        MY_DRIVER_LEVEL = c::net_driverlevel;

        // ADR-009: under -Duse_rust_cvar the `Cmd_AddCommand` name is itself
        // a Host_Reraise wrapper, so it must be reached through a guard.
        // COMPAT: registered BEFORE the safemode/-nolan early-out, so
        // `net_stats` exists even when the driver is disabled -- unlike
        // `test`/`test2` below.
        let st = NetDgrmOrch_Glue_AddNetStatsCommand();
        if st != 0 {
            return st;
        }

        if c::safemode != 0 || c::COM_CheckParm(c"-nolan".as_ptr()) != 0 {
            *out = -1;
            return 0;
        }

        let mut num_inited = 0;
        let mut i = 0;
        while i < net_numlandrivers {
            let ld = landriver(i);
            let csock = ((*ld).init.expect("landriver Init"))();
            if csock == INVALID_SOCKET {
                i += 1;
                continue;
            }
            // COMPAT: on failure the three fields keep their static-init
            // values; the C zeroes nothing here either.
            (*ld).initialized = true;
            (*ld).control_sock = csock;
            (*ld).listening_sock = INVALID_SOCKET;
            num_inited += 1;
            i += 1;
        }

        if num_inited == 0 {
            *out = -1;
            return 0;
        }

        // `test` then `test2`, in that order (net_dgrm.c:598-599)
        let st = NetDgrmOrch_Glue_AddTestCommands();
        if st != 0 {
            return st;
        }

        *out = 0;
        0
    }
}

// -----------------------------------------------------------------------------
// net_dgrm.c:604 -- Datagram_Shutdown
// -----------------------------------------------------------------------------

/// `void Datagram_Shutdown (void)` (net_dgrm.c:604). Nothing beneath it
/// raises (the landriver `Shutdown` slots reach `Sys_Error` at worst, which
/// aborts), so the glue wrapper is a plain pass-through.
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_shutdown() {
    // SAFETY: caller contract
    unsafe {
        // COMPAT: this resets heartbeat_time as a side effect (`:640`).
        datagram_listen(false);

        //
        // shutdown the lan drivers
        //
        let mut i = 0;
        while i < net_numlandrivers {
            let ld = landriver(i);
            if (*ld).initialized {
                ((*ld).shutdown.expect("landriver Shutdown"))();
                (*ld).initialized = false;
            }
            i += 1;
        }
    }
}

// -----------------------------------------------------------------------------
// net_dgrm.c:623 -- Datagram_Close
// -----------------------------------------------------------------------------

/// `void Datagram_Close (qsocket_t *sock)` (net_dgrm.c:623). `sfunc` is
/// `net_landrivers[sock->landriver]` (net_dgrm_int.h). No raise beneath it.
///
/// # Safety
/// Single-threaded host frame; `sock` is a live qsocket.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_close(sock: *mut QSocket) {
    // SAFETY: caller contract
    unsafe {
        if (*sock).isvirtual {
            (*sock).isvirtual = false;
            (*sock).socket = INVALID_SOCKET;
        } else {
            let ld = landriver((*sock).landriver);
            ((*ld).close_socket.expect("landriver Close_Socket"))((*sock).socket);
        }
    }
}

// -----------------------------------------------------------------------------
// net_dgrm.c:634 -- Datagram_Listen
// -----------------------------------------------------------------------------

/// `void Datagram_Listen (qboolean state)` (net_dgrm.c:634). The `Sys_Error`
/// at `:663` aborts rather than longjmps, so it is a plain extern and the
/// glue wrapper needs no `Host_Reraise` (contract, ADR-009 section).
///
/// # Safety
/// Single-threaded host frame.
unsafe fn datagram_listen(state: QBoolean) {
    // SAFETY: caller contract
    unsafe {
        let mut islistening = false;

        HEARTBEAT_TIME = 0.0; // reset it

        let mut i = 0;
        while i < net_numlandrivers {
            let ld = landriver(i);
            if (*ld).initialized {
                (*ld).listening_sock = ((*ld).listen.expect("landriver Listen"))(state);
                if (*ld).listening_sock != INVALID_SOCKET {
                    islistening = true;
                }

                // COMPAT: this walk of the whole active list runs once per
                // *initialized landriver*, redoing work it already did, and
                // it unvirtualises sockets belonging to other landrivers and
                // even when `state` is false. Loop nesting preserved
                // verbatim (net_dgrm.c:645-656).
                let mut s = c::net_activeSockets.cast::<QSocket>();
                while !s.is_null() {
                    if (*s).isvirtual {
                        (*s).isvirtual = false;
                        (*s).socket = INVALID_SOCKET;
                    }
                    s = (*s).next;
                }
            }
            i += 1;
        }

        if state && !islistening {
            if c::isDedicated {
                // COMPAT: a trailing "\n" inside a Sys_Error format, as in
                // the C. Sys_Error aborts; it does not longjmp.
                c::Sys_Error(c"Unable to open any listening sockets\n".as_ptr());
            }
            c::Con_Warning(c"Unable to open any listening sockets\n".as_ptr());
        }
    }
}

/// C ABI entry for the `Datagram_Listen` vtable slot.
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_listen(state: QBoolean) {
    // SAFETY: caller contract
    unsafe { datagram_listen(state) }
}

// -----------------------------------------------------------------------------
// net_dgrm.c:671 -- Datagram_Rcon_Flush
// -----------------------------------------------------------------------------

/// `void Datagram_Rcon_Flush (const char *text)` (net_dgrm.c:671) -- the
/// `Con_Redirect` callback agent B installs at `:936`. In no header;
/// referenced only inside net_dgrm.c. The glue keeps the exact C signature
/// and calling convention so the redirect machinery is untouched.
///
/// The packet is built in a local `quake_net::sizebuf::SizeBuf` over a stack
/// buffer -- the global `net_message` is never touched, and no C
/// `MSG_Write*`/`SZ_*` entry point is called (they `Host_Error`).
///
/// # Safety
/// Single-threaded host frame; `text` is NUL-terminated or null.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_rcon_flush(text: *const c_char) {
    // SAFETY: caller contract
    unsafe {
        let mut buffer = [0u8; 8192];
        let overflow_events;
        let overflowed;
        let cursize;
        {
            // C: msg.data = buffer; msg.maxsize = sizeof (buffer);
            //    msg.allowoverflow = true; SZ_Clear (&msg);
            // COMPAT: cursize/overflowed are uninitialised until SZ_Clear
            // writes them; SizeBuf::new + clear() reproduces that.
            let mut msg = SizeBuf::new(&mut buffer);
            msg.allowoverflow = true;
            msg.clear();

            // save space for the header, filled in later
            let r = msg::write_long(&mut msg, 0)
                .and_then(|_| msg::write_byte(&mut msg, CCREP_RCON as c_int))
                .and_then(|_| {
                    // C: MSG_WriteString (&msg, text). `None` is C's NULL
                    // branch (writes the terminator only); Con_Redirect never
                    // passes NULL, but the branch is kept.
                    msg::write_string(
                        &mut msg,
                        if text.is_null() {
                            None
                        } else {
                            Some(core::ffi::CStr::from_ptr(text).to_bytes())
                        },
                    )
                });
            if let Err(e) = r {
                sz_fatal(e);
            }

            overflow_events = msg.overflow_events;
            overflowed = msg.overflowed;
            cursize = msg.cursize;
        }
        // the sizebuf borrow of `buffer` has ended; Con_Printf is not a leaf,
        // so SZ_GetSpace's "overflow" diagnostics are emitted only now. This
        // function prints nothing else, so the console text is unchanged.
        for _ in 0..overflow_events {
            con_print_bytes(b"SZ_GetSpace: overflow\n");
        }

        if overflowed {
            // COMPAT: the whole rcon response is dropped, silently.
            return;
        }

        // *((int *)msg.data) = BigLong (NETFLAG_CTL | (msg.cursize & NETFLAG_LENGTH_MASK));
        let word = NETFLAG_CTL | ((cursize as u32) & NETFLAG_LENGTH_MASK);
        buffer[0..4].copy_from_slice(&word.to_be_bytes());

        // COMPAT: rcon_response_landriver is an index stored in a
        // sys_socket_t (see the static's doc). The C bounds-checks it
        // nowhere either (net_dgrm.c:687); `landriver` is raw-pointer
        // arithmetic, so this reproduces the C exactly, fault included.
        let ld = landriver(RCON_RESPONSE_LANDRIVER as c_int);
        ((*ld).write.expect("landriver Write"))(
            RCON_RESPONSE_SOCKET,
            buffer.as_mut_ptr(),
            cursize,
            &raw mut RCON_RESPONSE_ADDRESS,
        );
    }
}

/// `SZ_GetSpace`'s non-longjmp failure (net_msg.c:491). `Sys_Error` aborts.
fn sz_fatal(e: WireError) {
    match e {
        WireError::OversizeWrite => {
            // SAFETY: NUL-terminated literal format; Sys_Error is noreturn
            unsafe {
                c::Sys_Error(c"SZ_GetSpace: %i is > full buffer size".as_ptr(), 8192i32);
            }
        }
        // Unreachable: allowoverflow is true, so Overflow never fires, and
        // the two writes here (a long, and CCREP_RCON = 0x86) have no debug
        // range check that can trip.
        WireError::Overflow | WireError::RangeError(_) => {
            // SAFETY: as above
            unsafe {
                c::Sys_Error(c"Datagram_Rcon_Flush: unexpected sizebuf error".as_ptr());
            }
        }
    }
}

// ############################ agent B: server control packet ################
// ---------------------------------------------------------------------------
// server-state access
//
// `sv`/`svs` moved to Rust in T6.6, but they live in `quake-capi::sv_main`,
// which is gated on `feature = "host"` while this module is gated on
// `feature = "net"`. The `build-rs-chost` CI leg builds net-on/host-off, so
// the reference has to be cfg-split rather than a plain `use`: taking the
// `extern` arm unconditionally would put a declaration and a `#[no_mangle]`
// definition of the same symbol in one crate.
//
// `host_client` is defined in `Quake/host.c` in every configuration, so it is
// always an extern. (`sv_main.rs` declares its own private copy; a second
// declaration of an all-C symbol is fine.)
// ---------------------------------------------------------------------------

#[cfg(feature = "host")]
use crate::sv_main::{sv, svs};

#[cfg(not(feature = "host"))]
extern "C" {
    /// `Quake/server.h` -- `extern server_t sv;`
    static mut sv: Server;
    /// `Quake/server.h` -- `extern server_static_t svs;`
    static mut svs: ServerStatic;
}

/// `&sv` without forming a reference to the `static mut` (the
/// `static_mut_refs` lint). Same shape as `sv_main.rs`'s private `sv_p`.
#[inline]
fn sv_p() -> *mut Server {
    core::ptr::addr_of_mut!(sv)
}

/// `&svs`, as [`sv_p`].
#[inline]
fn svs_p() -> *mut ServerStatic {
    core::ptr::addr_of_mut!(svs)
}

/// Copies a C string out. `NULL` yields empty, which is what every call site
/// here wants: `MSG_WriteString (sb, NULL)` and `MSG_WriteString (sb, "")`
/// both emit a single NUL byte.
unsafe fn c_bytes(p: *const c_char) -> Vec<u8> {
    if p.is_null() {
        Vec::new()
    } else {
        // SAFETY: caller passes a live NUL-terminated engine string.
        unsafe { CStr::from_ptr(p).to_bytes().to_vec() }
    }
}

/// `strlen` over a fixed-size `char[N]` engine field.
///
/// Bound: the iterator is over the declared extent, so an unterminated field
/// truncates at N instead of running off the end the way C's `strlen` would.
/// Unreachable in practice -- every writer of `sv.name` / `client_t.name`
/// goes through `q_strlcpy`.
fn field_bytes(src: &[c_char]) -> Vec<u8> {
    src.iter()
        .take_while(|&&ch| ch != 0)
        .map(|&ch| ch as u8)
        .collect()
}

/// `strcpy (dst, src)` into a `qsocket_t` NET_NAMELEN address field
/// (`net_dgrm.c:1096-1097`).
///
/// Bound: every landriver's `AddrToString` returns its own
/// `static char buffer[64]` filled by `q_snprintf (buffer, sizeof (buffer),
/// ...)` (`net_udp.c:375-377`; `net_loop.c` returns the literal "LOCAL"), so
/// the source is at most 63 bytes plus its NUL and the copy always
/// terminates before the bound. The explicit `i < dst.len()` is the STOP B
/// item (nn) guard: it makes a hypothetical over-long driver string truncate
/// rather than abort on an implicit bounds check.
unsafe fn strcpy_field(dst: &mut [c_char; NET_NAMELEN], src: *const c_char) {
    let n = dst.len();
    let mut i = 0usize;
    while i < n {
        // SAFETY: caller passes the driver's NUL-terminated static buffer;
        // the loop stops at that NUL, and at n regardless.
        let b = unsafe { *src.add(i) };
        dst[i] = b;
        if b == 0 {
            return;
        }
        i += 1;
    }
}

/// Runs `f` over a borrowed `quake_net` view of the C `net_message`, writing
/// `cursize`/`overflowed` back afterwards -- also on the error path, matching
/// the partial effects C leaves behind when `Host_Error` fires mid-write.
///
/// This is a local copy of `net.rs`'s private `with_sizebuf` narrowed to
/// `net_message`; see the notes for the "lift it into a shared module"
/// question.
///
/// No C is called from inside `f` (see the ADR-009/aliasing note in
/// `net.rs`): every C-derived value a reply needs is gathered before the
/// borrow opens.
///
/// # Safety
/// Single-threaded host frame; `net_message` has been `SZ_Alloc`ed
/// (`NET_Init`).
unsafe fn with_net_message<R>(f: impl FnOnce(&mut SizeBuf<'_>) -> R) -> R {
    // SAFETY: caller contract above.
    unsafe {
        let raw = &raw mut c::net_message;
        let data = if (*raw).data.is_null() {
            &mut [][..]
        } else {
            core::slice::from_raw_parts_mut((*raw).data, (*raw).maxsize.max(0) as usize)
        };
        let mut view = SizeBuf {
            allowoverflow: (*raw).allowoverflow,
            overflowed: (*raw).overflowed,
            data,
            cursize: (*raw).cursize,
            overflow_events: 0,
        };
        let out = f(&mut view);
        (*raw).cursize = view.cursize;
        (*raw).overflowed = view.overflowed;
        // net_message.allowoverflow is false, so overflow_events is
        // unreachable and there is no deferred "SZ_GetSpace: overflow"
        // diagnostic to drain.
        out
    }
}

/// `SZ_Clear (&net_message)`.
///
/// # Safety
/// As [`with_net_message`].
unsafe fn net_message_clear() {
    // SAFETY: caller contract.
    unsafe { with_net_message(|sb| sb.clear()) }
}

/// `*((int *)net_message.data) = BigLong (NETFLAG_CTL | (net_message.cursize
/// & NETFLAG_LENGTH_MASK))`.
fn patch_ctl_header(sb: &mut SizeBuf<'_>) {
    let hdr = (NETFLAG_CTL | (sb.cursize as u32 & NETFLAG_LENGTH_MASK)).to_be_bytes();
    // Bound (STOP B item (nn)): every call site reaches here only after a
    // MSG_WriteLong(0) plus at least one further write succeeded against
    // this buffer, so data.len() >= cursize >= 5. The guard reproduces that
    // invariant explicitly instead of relying on an implicit bounds check
    // that `panic = "abort"` would turn into a process abort.
    if sb.data.len() >= 4 {
        sb.data[..4].copy_from_slice(&hdr);
    }
}

/// `dfunc.Write (acceptsock, net_message.data, net_message.cursize,
/// clientaddr)`. The return value is discarded, as in C.
///
/// # Safety
/// Single-threaded host frame; `clientaddr` is a live `struct qsockaddr`.
unsafe fn dfunc_write_net_message(acceptsock: SysSocket, clientaddr: *mut QSockAddr) {
    // SAFETY: vtable slot installed by net_bsd.c/net_win.c; the buffer is
    // net_message's own allocation and `cursize` bytes of it are initialized.
    unsafe {
        let ld = dfunc();
        let nm = &raw mut c::net_message;
        ((*ld).write.expect("landriver Write"))(acceptsock, (*nm).data, (*nm).cursize, clientaddr);
    }
}

// ---------------------------------------------------------------------------
// net_dgrm.c:689 -- _Datagram_ServerControlPacket
// ---------------------------------------------------------------------------

/// `net_dgrm.c:689` -- `static void _Datagram_ServerControlPacket
/// (sys_socket_t acceptsock, struct qsockaddr *clientaddr, byte *data,
/// unsigned int length)`.
///
/// Returns a `Host_Guard` status (`HOST_GUARD_OK` / `HOST_GUARD_ABORTSERVER`
/// / `HOST_GUARD_SCREEN_ERROR`) or [`RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW`].
/// The only caller is `Datagram_GetAnyMessage` (agent A), which must
/// propagate a non-zero status straight out to its C vtable wrapper --
/// ADR-009: the re-raise happens there, never here.
///
/// # Safety
/// Single-threaded host frame. `data` points at `packetBuffer` (the C
/// `dgrm_packet_t`, `sizeof == NET_DATAGRAMSIZE`) holding at least `length`
/// bytes, with `length >= 4` (the caller's `if (length < 4) continue;`).
/// `clientaddr` is a live `struct qsockaddr`.
#[allow(clippy::too_many_lines)]
#[allow(clippy::collapsible_if)] // transliterating :1016/:1022 as written
unsafe fn server_control_packet(
    acceptsock: SysSocket,
    clientaddr: *mut QSockAddr,
    data: *mut u8,
    length: c_uint,
) -> c_int {
    // SAFETY: caller contract above; every engine global touched here is
    // host-frame-only, and no C is called while a Rust borrow of
    // net_message is open.
    unsafe {
        // `control = BigLong (*((int *)data));` -- length >= 4 per contract.
        let control = i32::from_be_bytes([*data, *data.add(1), *data.add(2), *data.add(3)]);

        if control == -1 {
            if sv_public.value == 0.0 {
                return HOST_GUARD_OK;
            }

            // COMPAT (preserved C defect, net_dgrm.c:705): `data[length] = 0`
            // writes ONE PAST the end of `packetBuffer` when the landriver
            // filled it completely -- `Read` is capped at NET_DATAGRAMSIZE,
            // which is exactly `sizeof packetBuffer`. Transliterated through
            // a raw pointer so the Rust build behaves as the C build does
            // instead of aborting on a bounds check; see the notes, this is
            // a live upstream bug and not something this port may fix.
            *data.add(length as usize) = 0;
            Cmd_TokenizeString(data.add(4).cast::<c_char>());

            let argv0 = c_bytes(c::Cmd_Argv(0));
            if argv0 == b"getinfo" || argv0 == b"getstatus" {
                // master, as well as other clients, may send us one of these
                // two packets to get our serverinfo data
                let full = argv0 == b"getstatus";
                let args = c::host_cmd::Cmd_Args();
                let gamedir = c::sv_main::COM_GetGameNames(false);
                let mut numclients: c_uint = 0;
                let mut numbots: c_uint = 0;

                // `q_strlcpy (cookie, str, sizeof (cookie))` into
                // `char cookie[128]`: truncate to 127 bytes + NUL.
                let mut cookie = if args.is_null() {
                    Vec::new()
                } else {
                    c_bytes(args)
                };
                cookie.truncate(127);

                let clients = (*svs_p()).clients;
                let maxclients = (*svs_p()).maxclients;
                for i in 0..maxclients {
                    // Bound: `clients` is the svs.maxclients-element array
                    // Host_InitLocal allocated; C's own loop bound.
                    let cl = clients.add(i as usize);
                    if (*cl).active {
                        numclients += 1;
                        if (*cl).netconnection.is_null() {
                            numbots += 1;
                        }
                    }
                }

                // Every `MSG_WriteString (&net_message, s)` at :734-804 is
                // followed by `net_message.cursize--`, i.e. the NUL is
                // dropped again and the payload is the raw concatenation.
                // Gather the arguments first (each `va` result is copied out
                // immediately -- va's ring is only VA_NUM_BUFFS deep and the
                // getstatus reply makes more calls than that), then do the
                // writes in one borrow.
                let mut parts: Vec<Vec<u8>> = Vec::new();
                parts.push(if full {
                    b"statusResponse\n".to_vec()
                } else {
                    b"infoResponse\n".to_vec()
                });

                c::COM_Parse(com_protocolname.string);
                let com_token = c::COM_ThreadToken();
                if *com_token != 0 {
                    // the master server needs this. This tells the master
                    // which game we should be listed as.
                    parts.push(c_bytes(c::cl_main::va(
                        c"\\gamename\\%s".as_ptr(),
                        com_token,
                    )));
                }
                parts.push(b"\\protocol\\3".to_vec());
                // this is stupid
                parts.push(engine_name_and_ver_field());
                parts.push(c_bytes(c::cl_main::va(
                    c"\\nqprotocol\\%u".as_ptr(),
                    (*sv_p()).protocol,
                )));
                if *gamedir != 0 {
                    parts.push(c_bytes(c::cl_main::va(c"\\modname\\%s".as_ptr(), gamedir)));
                }
                if (*sv_p()).name[0] != 0 {
                    parts.push(c_bytes(c::cl_main::va(
                        c"\\mapname\\%s".as_ptr(),
                        (&raw const (*sv_p()).name).cast::<c_char>(),
                    )));
                }
                if *c::sv_main::deathmatch.string != 0 {
                    parts.push(c_bytes(c::cl_main::va(
                        c"\\deathmatch\\%s".as_ptr(),
                        c::sv_main::deathmatch.string,
                    )));
                }
                if *c::progs_builtins_sv::teamplay.string != 0 {
                    parts.push(c_bytes(c::cl_main::va(
                        c"\\teamplay\\%s".as_ptr(),
                        c::progs_builtins_sv::teamplay.string,
                    )));
                }
                if *c::sv_main::hostname.string != 0 {
                    parts.push(c_bytes(c::cl_main::va(
                        c"\\hostname\\%s".as_ptr(),
                        c::sv_main::hostname.string,
                    )));
                }
                parts.push(c_bytes(c::cl_main::va(
                    c"\\clients\\%u".as_ptr(),
                    numclients,
                )));
                if numbots != 0 {
                    parts.push(c_bytes(c::cl_main::va(c"\\bots\\%u".as_ptr(), numbots)));
                }
                parts.push(c_bytes(c::cl_main::va(
                    c"\\sv_maxclients\\%i".as_ptr(),
                    maxclients,
                )));
                if !cookie.is_empty() {
                    let mut cz = cookie.clone();
                    cz.push(0);
                    parts.push(c_bytes(c::cl_main::va(
                        c"\\challenge\\%s".as_ptr(),
                        cz.as_ptr().cast::<c_char>(),
                    )));
                }

                if full {
                    for i in 0..maxclients {
                        // Bound: as the counting loop above.
                        let cl = clients.add(i as usize);
                        if (*cl).active {
                            let mut total: f32 = 0.0;
                            // Bound: ping_times is [f32; NUM_PING_TIMES].
                            for j in 0..NUM_PING_TIMES {
                                total += (*cl).ping_times[j];
                            }
                            total /= NUM_PING_TIMES as f32;
                            total *= 1000.0; // put it in ms
                            parts.push(c_bytes(c::cl_main::va(
                                c"\n%i %i %i_%i \"%s\"".as_ptr(),
                                (*cl).old_frags,
                                // COMPAT: ADR-010 -- `(int)total`. Rust `as`
                                // saturates where C's float-to-int
                                // conversion is UB out of range / on NaN.
                                total as c_int,
                                (*cl).colors & 15,
                                (*cl).colors >> 4,
                                (*cl).name.as_ptr(),
                            )));
                        }
                    }
                }

                let r = with_net_message(|sb| {
                    sb.clear();
                    msg::write_long(sb, -1)?;
                    for p in &parts {
                        msg::write_string(sb, Some(p.as_slice()))?;
                        sb.cursize -= 1;
                    }
                    Ok::<(), WireError>(())
                });
                if r.is_err() {
                    return RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW;
                }
                // note: no CTL header patch here -- the MSG_WriteLong(-1)
                // above IS this reply's header (the dpmaster format).
                dfunc_write_net_message(acceptsock, clientaddr);
                net_message_clear();
            }
            return HOST_GUARD_OK;
        }

        // `(control & (~NETFLAG_LENGTH_MASK)) != (int)NETFLAG_CTL`
        if (control & !(NETFLAG_LENGTH_MASK as i32)) != NETFLAG_CTL as i32 {
            return HOST_GUARD_OK;
        }
        // `(control & NETFLAG_LENGTH_MASK) != length` -- C promotes the int
        // to unsigned for the comparison; the masked value is non-negative,
        // so the promotion is value-preserving.
        if (control as u32 & NETFLAG_LENGTH_MASK) != length {
            return HOST_GUARD_OK;
        }

        // sigh... FIXME: potentially abusive memcpy
        //
        // This is reachable overflow, not a theoretical one: `length` can be
        // up to NET_DATAGRAMSIZE (64008) while net_message is NET_MAXMESSAGE
        // (64000), so a hostile packet with a matching header raises the
        // SZ_GetSpace Host_Error. Preserved.
        let packet = core::slice::from_raw_parts(data.cast_const(), length as usize);
        let r = with_net_message(|sb| {
            sb.clear();
            sb.write(packet)
        });
        if r.is_err() {
            return RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW;
        }

        crate::net::MSG_BeginReading();
        crate::net::MSG_ReadLong();

        let command = crate::net::MSG_ReadByte();

        if command == CCREQ_SERVER_INFO as c_int {
            if c_bytes(crate::net::MSG_ReadString()) != b"QUAKE" {
                return HOST_GUARD_OK;
            }

            let ld = dfunc();
            let mut newaddr: QSockAddr = core::mem::zeroed();
            ((*ld).get_socket_addr.expect("landriver GetSocketAddr"))(acceptsock, &mut newaddr);
            let addrstr = c_bytes(((*ld).addr_to_string.expect("landriver AddrToString"))(
                &mut newaddr,
                false,
            ));
            let hostname_s = c_bytes(c::sv_main::hostname.string);
            let svname = field_bytes(&(*sv_p()).name);
            let activeconnections = c::net_activeconnections;
            let maxclients = (*svs_p()).maxclients;

            let r = with_net_message(|sb| {
                sb.clear();
                // save space for the header, filled in later
                msg::write_long(sb, 0)?;
                msg::write_byte(sb, CCREP_SERVER_INFO as c_int)?;
                msg::write_string(sb, Some(addrstr.as_slice()))?;
                msg::write_string(sb, Some(hostname_s.as_slice()))?;
                msg::write_string(sb, Some(svname.as_slice()))?;
                msg::write_byte(sb, activeconnections)?;
                msg::write_byte(sb, maxclients)?;
                msg::write_byte(sb, NET_PROTOCOL_VERSION as c_int)?;
                patch_ctl_header(sb);
                Ok::<(), WireError>(())
            });
            if r.is_err() {
                return RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW;
            }
            dfunc_write_net_message(acceptsock, clientaddr);
            net_message_clear();
            return HOST_GUARD_OK;
        }

        if command == CCREQ_PLAYER_INFO as c_int {
            let player_number = crate::net::MSG_ReadByte();
            let mut active_number: c_int = -1;

            let maxclients = (*svs_p()).maxclients;
            let mut client_number: c_int = 0;
            let mut client = (*svs_p()).clients;
            // `for (clientNumber = 0, client = svs.clients; clientNumber <
            //  svs.maxclients; clientNumber++, client++)` -- the `break`
            // fires before the increment, so `client` still points at the
            // matched slot.
            while client_number < maxclients {
                if (*client).active {
                    active_number += 1;
                    if active_number == player_number {
                        break;
                    }
                }
                client_number += 1;
                client = client.add(1);
            }

            if client_number == maxclients {
                return HOST_GUARD_OK;
            }
            // Bound: `client` is only dereferenced below, and reaching here
            // means the loop broke with client_number < maxclients, i.e.
            // `client` is inside the svs.clients array. (On the normal exit
            // it is the one-past-the-end pointer C also forms, and that
            // path returned above.)

            let name = field_bytes(&(*client).name);
            let colors = (*client).colors;
            // COMPAT: ADR-010 -- `(int)client->edict->v.frags`, saturating.
            let frags = (*(*client).edict).v.frags as c_int;
            let (ping_or_zero, addr) = if (*client).netconnection.is_null() {
                (0, b"Bot".to_vec())
            } else {
                (
                    // COMPAT: ADR-010 -- `(int)(net_time -
                    // client->netconnection->connecttime)`, saturating.
                    (c::net_time - (*(*client).netconnection).connecttime) as c_int,
                    c_bytes(c::host_cmd::NET_QSocketGetMaskedAddressString(
                        (*client).netconnection.cast(),
                    )),
                )
            };

            let r = with_net_message(|sb| {
                sb.clear();
                // save space for the header, filled in later
                msg::write_long(sb, 0)?;
                msg::write_byte(sb, CCREP_PLAYER_INFO as c_int)?;
                msg::write_byte(sb, player_number)?;
                msg::write_string(sb, Some(name.as_slice()))?;
                msg::write_long(sb, colors)?;
                msg::write_long(sb, frags)?;
                msg::write_long(sb, ping_or_zero)?;
                msg::write_string(sb, Some(addr.as_slice()))?;
                patch_ctl_header(sb);
                Ok::<(), WireError>(())
            });
            if r.is_err() {
                return RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW;
            }
            dfunc_write_net_message(acceptsock, clientaddr);
            net_message_clear();
            return HOST_GUARD_OK;
        }

        if command == CCREQ_RULE_INFO as c_int {
            // find the search start location
            let prev_cvar_name = crate::net::MSG_ReadString();
            let var = Cvar_FindVarAfter(prev_cvar_name, c::cvarflags_t_CVAR_SERVERINFO);
            let named = if var.is_null() {
                None
            } else {
                Some((c_bytes((*var).name), c_bytes((*var).string)))
            };

            // send the response
            let r = with_net_message(|sb| {
                sb.clear();
                // save space for the header, filled in later
                msg::write_long(sb, 0)?;
                msg::write_byte(sb, CCREP_RULE_INFO as c_int)?;
                if let Some((n, s)) = &named {
                    msg::write_string(sb, Some(n.as_slice()))?;
                    msg::write_string(sb, Some(s.as_slice()))?;
                }
                patch_ctl_header(sb);
                Ok::<(), WireError>(())
            });
            if r.is_err() {
                return RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW;
            }
            dfunc_write_net_message(acceptsock, clientaddr);
            net_message_clear();
            return HOST_GUARD_OK;
        }

        if command == CCREQ_RCON as c_int {
            // FIXME: this really needs crypto
            let password = c_bytes(crate::net::MSG_ReadString());

            RCON_RESPONSE_ADDRESS = *clientaddr;
            RCON_RESPONSE_SOCKET = acceptsock;
            RCON_RESPONSE_LANDRIVER = NET_LANDRIVERLEVEL as SysSocket;

            let configured = c_bytes(rcon_password.string);
            let response: &[u8] = if configured.is_empty() {
                b"rcon is not enabled on this server"
            } else if password == configured {
                Con_Redirect(Some(Datagram_Rcon_Flush));
                let cmd = crate::net::MSG_ReadString();
                let status = NetDgrmOrch_Glue_ExecuteString(cmd);
                if status != HOST_GUARD_OK {
                    // COMPAT (net_dgrm.c:935-938): C's Cmd_ExecuteString
                    // longjmps straight past `Con_Redirect (NULL)`, leaving
                    // the rcon redirect installed with its buffer unflushed
                    // -- the next Con_Redirect call flushes it to this (now
                    // stale) rcon address. Returning the caught status here
                    // without clearing the redirect reproduces that state
                    // exactly; the caller must propagate it untouched so the
                    // C vtable wrapper re-raises (ADR-009).
                    return status;
                }
                Con_Redirect(None);
                return HOST_GUARD_OK;
            } else if password == b"password" {
                b"What, you really thought that would work? Seriously?"
            } else if password == b"thebackdoor" {
                b"Oh look! You found the backdoor. Don't let it slam you in the face on your way out."
            } else {
                b"Your password is just WRONG dude."
            };

            let mut resp = response.to_vec();
            resp.push(0);
            Datagram_Rcon_Flush(resp.as_ptr().cast::<c_char>());
            return HOST_GUARD_OK;
        }

        if command != CCREQ_CONNECT as c_int {
            return HOST_GUARD_OK;
        }

        if c_bytes(crate::net::MSG_ReadString()) != b"QUAKE" {
            return HOST_GUARD_OK;
        }

        if crate::net::MSG_ReadByte() != NET_PROTOCOL_VERSION as c_int {
            let r = with_net_message(|sb| {
                sb.clear();
                // save space for the header, filled in later
                msg::write_long(sb, 0)?;
                msg::write_byte(sb, CCREP_REJECT as c_int)?;
                msg::write_string(sb, Some(&b"Incompatible version.\n"[..]))?;
                patch_ctl_header(sb);
                Ok::<(), WireError>(())
            });
            if r.is_err() {
                return RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW;
            }
            dfunc_write_net_message(acceptsock, clientaddr);
            net_message_clear();
            return HOST_GUARD_OK;
        }

        // read proquake extensions
        let mut proquake_mod = crate::net::MSG_ReadByte();
        if c::msg_badread {
            proquake_mod = 0;
        }
        // net_dgrm.c:974-984 is `#if 0` and :986-1007 is `#ifdef BAN_TEST`
        // (undefined at :23) -- neither is compiled, so neither is ported.

        let ld = dfunc();

        // see if this guy is already connected
        let mut s = c::net_activeSockets.cast::<QSocket>();
        while !s.is_null() {
            if (*s).driver != c::net_driverlevel {
                s = (*s).next;
                continue;
            }
            if (*s).disconnected {
                s = (*s).next;
                continue;
            }
            let ret = ((*ld).addr_compare.expect("landriver AddrCompare"))(
                clientaddr,
                &raw mut (*s).addr,
            );
            if ret == 0 {
                // is this a duplicate connection reqeust?
                //
                // COMPAT: the `ret == 0 &&` half of :1022 is dead -- the
                // enclosing `if` already tested it. Preserved.
                if ret == 0 && c::net_time - (*s).connecttime < 2.0 {
                    // yes, so send a duplicate reply
                    let mut newaddr: QSockAddr = core::mem::zeroed();
                    ((*ld).get_socket_addr.expect("landriver GetSocketAddr"))(
                        (*s).socket,
                        &mut newaddr,
                    );
                    let port =
                        ((*ld).get_socket_port.expect("landriver GetSocketPort"))(&mut newaddr);
                    let proquake = (*s).proquake_angle_hack;

                    let r = with_net_message(|sb| {
                        sb.clear();
                        // save space for the header, filled in later
                        msg::write_long(sb, 0)?;
                        msg::write_byte(sb, CCREP_ACCEPT as c_int)?;
                        msg::write_long(sb, port)?;
                        if proquake {
                            // proquake
                            msg::write_byte(sb, 1)?;
                            // ver 30 should be safe. 34 screws with our
                            // single-server-socket stuff.
                            msg::write_byte(sb, 30)?;
                            msg::write_byte(sb, 0)?; // no flags
                        }
                        patch_ctl_header(sb);
                        Ok::<(), WireError>(())
                    });
                    if r.is_err() {
                        return RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW;
                    }
                    dfunc_write_net_message(acceptsock, clientaddr);
                    net_message_clear();
                    return HOST_GUARD_OK;
                }

                // it's somebody coming back in from a crash/disconnect
                // so close the old qsocket and let their retry get them back
                // in (see the FIXME block at net_dgrm.c:1042-1052)
                //
                // The whole scan stays in C. It is a raise frame -- SV_DropClient
                // reaches PR_ExecuteProgram of ClientDisconnect (host.c:590) --
                // and svs/host_client live behind `feature = "host"` while this
                // module is behind `feature = "net"`, which the build-rs-chost
                // leg (net on, host off) would otherwise break. `close_first`
                // selects the NET_Close-before-drop variant C does here and not
                // at :213; the close sits inside the match on both sides.
                let status = NetDgrmOrch_Glue_DropClient(s.cast(), true);
                if status != HOST_GUARD_OK {
                    // ADR-009: C longjmps out of SV_DropClient here, skipping
                    // the `break` and the `return` below.
                    return status;
                }
                return HOST_GUARD_OK;
            }
            s = (*s).next;
        }

        // find a free player slot
        let clients = (*svs_p()).clients;
        let maxclients = (*svs_p()).maxclients;
        let mut plnum: c_int = 0;
        while plnum < maxclients {
            // Bound: C's own `plnum < svs.maxclients`.
            if !(*clients.add(plnum as usize)).active {
                break;
            }
            plnum += 1;
        }
        let sock = if plnum < maxclients {
            c::NET_NewQSocket().cast::<QSocket>()
        } else {
            // can happen due to botclients.
            core::ptr::null_mut()
        };

        if sock.is_null() {
            // no room; try to let him know
            let r = with_net_message(|sb| {
                sb.clear();
                // save space for the header, filled in later
                msg::write_long(sb, 0)?;
                msg::write_byte(sb, CCREP_REJECT as c_int)?;
                msg::write_string(sb, Some(&b"Server is full.\n"[..]))?;
                patch_ctl_header(sb);
                Ok::<(), WireError>(())
            });
            if r.is_err() {
                return RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW;
            }
            dfunc_write_net_message(acceptsock, clientaddr);
            net_message_clear();
            return HOST_GUARD_OK;
        }

        (*sock).proquake_angle_hack = proquake_mod == 1;

        // everything is allocated, just fill in the details
        (*sock).isvirtual = true;
        (*sock).socket = acceptsock;
        (*sock).landriver = NET_LANDRIVERLEVEL;
        (*sock).addr = *clientaddr;
        // Both strcpys read the driver's one shared static buffer, so the
        // first copy must complete before the second AddrToString call.
        let true_addr = ((*ld).addr_to_string.expect("landriver AddrToString"))(clientaddr, false);
        strcpy_field(&mut (*sock).trueaddress, true_addr);
        let masked_addr = ((*ld).addr_to_string.expect("landriver AddrToString"))(clientaddr, true);
        strcpy_field(&mut (*sock).maskedaddress, masked_addr);

        // send him back the info about the server connection he has been
        // allocated
        let mut newaddr: QSockAddr = core::mem::zeroed();
        ((*ld).get_socket_addr.expect("landriver GetSocketAddr"))((*sock).socket, &mut newaddr);
        let port = ((*ld).get_socket_port.expect("landriver GetSocketPort"))(&mut newaddr);
        let proquake = (*sock).proquake_angle_hack;

        let r = with_net_message(|sb| {
            sb.clear();
            // save space for the header, filled in later
            msg::write_long(sb, 0)?;
            msg::write_byte(sb, CCREP_ACCEPT as c_int)?;
            msg::write_long(sb, port)?;
            if proquake {
                // proquake
                msg::write_byte(sb, 1)?;
                // ver 30 should be safe. 34 screws with our
                // single-server-socket stuff.
                msg::write_byte(sb, 30)?;
                msg::write_byte(sb, 0)?;
            }
            patch_ctl_header(sb);
            Ok::<(), WireError>(())
        });
        if r.is_err() {
            return RUST_DGRM_ORCH_NET_MESSAGE_OVERFLOW;
        }
        dfunc_write_net_message(acceptsock, clientaddr);
        net_message_clear();

        // spawn the client. (see the FIXME at net_dgrm.c:1118)
        //
        // Bound: `plnum < maxclients` here -- the `sock == NULL` early
        // return above covers the `plnum == maxclients` case
        // (net_dgrm.c:1071-1087).
        (*clients.add(plnum as usize)).netconnection = sock;
        NetDgrmOrch_Glue_ConnectClient(plnum)
    }
}

/// `"\ver\\" ENGINE_NAME_AND_VER` (`net_dgrm.c:744`). The macro bakes in a
/// build-date token only the C preprocessor has, so the glue exports the
/// finished literal as data and this copies it out -- exactly the treatment
/// `host_cmd.c:931`'s version line already gets
/// (`HostCmd_EngineVersionLine`, `quake-c-sys/src/host_cmd.rs:187`).
fn engine_name_and_ver_field() -> Vec<u8> {
    // SAFETY: a `const char *const` pointing at a string literal.
    unsafe { c_bytes(NetDgrmOrch_GetInfoVerField) }
}

// ---------------------------------------------------------------------------
// net_dgrm.c:1124 -- Datagram_CheckNewConnections
// ---------------------------------------------------------------------------

/// `net_dgrm.c:1124` -- `qsocket_t *Datagram_CheckNewConnections (void)`.
/// Only needs to do master stuff now, and always yields NULL.
///
/// The status is always `HOST_GUARD_OK`: nothing in the body can raise (the
/// heartbeat write goes straight to the landriver vtable, and `Con_Printf`
/// is treated as a non-raising leaf here, exactly as `net_main.rs` and
/// `net_dgrm.rs` already treat it). The `int` return and the
/// `Host_Reraise` wrapper are kept anyway so all ten `Datagram_*` entry
/// points have the one uniform shape.
///
/// # Safety
/// Single-threaded host frame; `out` is a valid out pointer.
#[allow(clippy::collapsible_if)] // transliterating :1127/:1129 as written
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_check_new_connections(out: *mut *mut QSocket) -> c_int {
    // SAFETY: caller contract above; all globals touched are host-frame-only.
    unsafe {
        if sv_public.value > 0.0 {
            if c::Sys_DoubleTime() > HEARTBEAT_TIME {
                // darkplaces here refers to the master server protocol,
                // rather than the game protocol (specifies that the server
                // responds to infoRequest packets from the master)
                //
                // `char str[] = "\377\377\377\377heartbeat DarkPlaces\n"` --
                // 25 bytes, no interior NUL, so `strlen (str) == 25`.
                let mut str_buf: [u8; 25] = *b"\xff\xff\xff\xffheartbeat DarkPlaces\n";
                let mut addr: QSockAddr = core::mem::zeroed();
                HEARTBEAT_TIME = c::Sys_DoubleTime() + 300.0;

                // `for (k = 0; net_masters[k].string; k++)`
                //
                // Bound (STOP B item (nn)): net_masters is the 8-element,
                // NULL-`string`-terminated cvar_t array at net_dgrm.c:47-55.
                // The sentinel stops the walk at k == 7; the explicit
                // `k < NET_MASTERS_COUNT` makes that a bound rather than a
                // trusted invariant.
                for k in 0..NET_MASTERS_COUNT {
                    let m = (&raw mut net_masters).cast::<c::cvar_t>().add(k);
                    if (*m).string.is_null() {
                        break;
                    }
                    if *(*m).string == 0 {
                        continue;
                    }
                    NET_LANDRIVERLEVEL = 0;
                    while NET_LANDRIVERLEVEL < net_numlandrivers {
                        let ld = landriver(NET_LANDRIVERLEVEL);
                        if (*ld).initialized && (*ld).listening_sock != INVALID_SOCKET {
                            if ((*ld).get_addr_from_name.expect("landriver GetAddrFromName"))(
                                (*m).string,
                                &mut addr,
                            ) >= 0
                            {
                                if sv_reportheartbeats.value != 0.0 {
                                    c::Con_Printf(
                                        c"Sending heartbeat to %s\n".as_ptr(),
                                        (*m).string,
                                    );
                                }
                                ((*ld).write.expect("landriver Write"))(
                                    (*ld).listening_sock,
                                    str_buf.as_mut_ptr(),
                                    str_buf.len() as c_int,
                                    &mut addr,
                                );
                            } else if sv_reportheartbeats.value != 0.0 {
                                c::Con_Printf(c"Unable to resolve %s\n".as_ptr(), (*m).string);
                            }
                        }
                        NET_LANDRIVERLEVEL += 1;
                    }
                }
                // C leaves net_landriverlevel == net_numlandrivers on exit;
                // preserved (the next driver loop resets it).
            }
        }

        *out = core::ptr::null_mut();
        HOST_GUARD_OK
    }
}

// ############################ agent C1: server discovery ####################

/// `enum slistScope_e { SLIST_LOOP, SLIST_LAN, SLIST_INTERNET }` (net.h:108).
/// Only the third member is tested here; net_main.c owns the variable.
const SLIST_INTERNET: c_int = 2;

// `hostcache_t hostcache[HOSTCACHESIZE]` (net_defs.h) -- a complete array
// type in C, so the extern can describe it truthfully. Same declaration as
// `net_main.rs:29`: separate modules, so two declarations of one symbol.
extern "C" {
    static mut hostcache: [HostCache; HOSTCACHESIZE];
}

/// `hostcache[idx]`, base pointer taken over the whole declared array so the
/// preserved C string overflows below (see `star_prefix_name`) carry
/// provenance over the entire `hostcache_t`, exactly as the C compiler's do.
/// Mirrors `net_main.rs:43`.
fn host(idx: usize) -> *mut HostCache {
    // SAFETY: every call site bounds idx by hostCacheCount, which this file
    // never lets exceed HOSTCACHESIZE -- the declared extent of the array
    unsafe { (&raw mut hostcache).cast::<HostCache>().add(idx) }
}

/// Pointer to one `hostcache_t` field, derived from the *entry* base rather
/// than from the field, so a write that runs past the field's own extent
/// still has provenance over the struct it lands in (ADR-004). The C's
/// `strcat` at :1406 does exactly that.
macro_rules! hc_field {
    ($h:expr, $f:ident) => {
        // SAFETY: $h is a live hostcache_t from `host()`; offset_of! is
        // inside the object
        $h.cast::<c_char>()
            .add(core::mem::offset_of!(HostCache, $f))
    };
}

/// `addr->qsa_family = fam` through the platform's family-field width.
///
/// A local copy of `quake_net::udp::set_family`: that module is `#[cfg(unix)]`
/// and so is not reachable on the Windows leg, which this file must build on.
fn set_family(addr: &mut QSockAddr, fam: c_int) {
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    {
        addr.qsa_family = fam as u8;
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    )))]
    {
        addr.qsa_family = fam as i16;
    }
}

/// C `memcmp (a, b, sizeof (struct qsockaddr))`. `QSockAddr` is `#[repr(C)]`
/// POD whose 64 bytes carry no padding on any supported target (u8+u8 or i16
/// header, then `[u8; 62]`), so the byte view is the same object the C
/// compares.
fn qsockaddr_bytes(a: &QSockAddr) -> &[u8] {
    // SAFETY: see the doc comment -- POD, no padding, no interior mutability
    unsafe {
        core::slice::from_raw_parts(
            (a as *const QSockAddr).cast::<u8>(),
            core::mem::size_of::<QSockAddr>(),
        )
    }
}

/// One entry of the anonymous file-scope `static struct { ... } *hostlist`
/// (net_dgrm.c:1186). Layout is not observed by C -- the whole allocation
/// moves to Rust with the storage split -- so this is a plain Rust struct.
#[derive(Clone, Copy)]
struct HostListEntry {
    driver: c_int,
    requery: QBoolean,
    master: QBoolean,
    addr: QSockAddr,
}

/// `hostlist` / `hostlist_count` / `hostlist_max` (net_dgrm.c:1186-1193).
///
/// The C grows this with `Mem_Realloc` in steps of 16 and never frees it;
/// a `Vec` is the same lifetime with a different growth curve. Nothing
/// outside net_dgrm.c reads `hostlist_count` / `hostlist_max` (they are
/// non-static only by omission), so the capacity policy is unobservable.
static mut HOSTLIST: Vec<HostListEntry> = Vec::new();

/// `hostlist`, as a slice. Single-threaded host frame, like every other
/// module static in the net files.
unsafe fn hostlist() -> &'static mut Vec<HostListEntry> {
    let p = &raw mut HOSTLIST;
    // SAFETY: host thread only; no other borrow is live across these uses
    unsafe { &mut *p }
}

/// C `SZ_Clear (&net_message)` without the raising C entry point.
///
/// # Safety
/// Single-threaded host frame.
unsafe fn sz_clear_net_message() {
    // SAFETY: caller contract
    unsafe {
        let p = &raw mut c::net_message;
        let raw = &mut *p;
        raw.cursize = 0;
        raw.overflowed = false;
    }
}

/// The `CCREQ_SERVER_INFO` control packet, built into `net_message` and
/// header-patched, shared by `_Datagram_SendServerQuery`'s non-master branch
/// (:1174-1180) and `_Datagram_SearchForHosts`'s broadcast (:1269-1275).
/// Byte-for-byte the same six statements in both places in the C.
///
/// # Safety
/// Single-threaded host frame.
unsafe fn build_server_info_request() {
    // SAFETY: caller contract
    unsafe {
        sz_clear_net_message();
        // Unreachable: the SZ_Clear above puts cursize at 0 and these four
        // writes total twelve bytes into net_message's NET_MAXMESSAGE
        // allocation, so C's SZ_GetSpace Host_Error has no way to fire. Both
        // callers are void and have no status channel to raise through.
        let _ = with_net_message(|b| {
            // save space for the header, filled in later
            msg::write_long(b, 0)?;
            msg::write_byte(b, CCREQ_SERVER_INFO as c_int)?;
            msg::write_string(b, Some(b"QUAKE"))?;
            msg::write_byte(b, NET_PROTOCOL_VERSION as c_int)
        });
        // *((int *)net_message.data) = BigLong (NETFLAG_CTL | (cursize & mask))
        let p = &raw mut c::net_message;
        let raw = &mut *p;
        let hdr = NETFLAG_CTL | (raw.cursize as u32 & NETFLAG_LENGTH_MASK);
        // BigLong is host->big; storing the big-endian bytes is the same
        // four bytes the C `int` store leaves behind
        core::ptr::copy_nonoverlapping(hdr.to_be_bytes().as_ptr(), raw.data, 4);
    }
}

/// `_Datagram_SendServerQuery` (net_dgrm.c:1166)
///
/// # Safety
/// Single-threaded host frame; the landriver vtable is installed.
unsafe fn datagram_send_server_query(addr: &QSockAddr, master: QBoolean) {
    // SAFETY: caller contract
    unsafe {
        if master
        // assume false if you want only the protocol 15 servers.
        {
            sz_clear_net_message();
            // Unreachable for the same reason as `build_server_info_request`:
            // twelve bytes into a freshly cleared 64000-byte net_message, in a
            // void function with no status channel.
            let _ = with_net_message(|b| {
                msg::write_long(b, !0)?;
                msg::write_string(b, Some(b"getinfo"))
            });
        } else {
            build_server_info_request();
        }

        let ld = dfunc();
        let raw = &raw const c::net_message;
        let mut a = *addr;
        ((*ld).write.expect("landriver Write"))(
            (*ld).control_sock,
            (*raw).data,
            (*raw).cursize,
            &mut a,
        );
        sz_clear_net_message();
    }
}

/// `_Datagram_AddPossibleHost` (net_dgrm.c:1195)
///
/// # Safety
/// Single-threaded host frame.
unsafe fn datagram_add_possible_host(addr: &QSockAddr, master: QBoolean) {
    // SAFETY: caller contract
    unsafe {
        let level = NET_LANDRIVERLEVEL;
        let list = hostlist();
        for e in list.iter() {
            if qsockaddr_bytes(&e.addr) == qsockaddr_bytes(addr) && e.driver == level {
                // we already know about it. it must have come from some other
                // master. don't respam.
                return;
            }
        }
        list.push(HostListEntry {
            driver: level,
            requery: true,
            master,
            addr: *addr,
        });
    }
}

/// `Info_ReadKey` (net_dgrm.c:1217).
///
/// `info` is the remote infostring with the slice end standing in for the C
/// string's NUL; `out` is the fixed destination field, `out.len()` its
/// `outsize`.
fn info_read_key(info: &[u8], key: &[u8], out: &mut [c_char]) {
    if out.is_empty() {
        // COMPAT / preserved defect: with outsize 0 the C computes
        // `e = out + outsize - 1` (one before the object) and then still runs
        // `*o++ = 0`, a one-byte underwrite. No net_dgrm.c call site passes 0,
        // so returning here cannot diverge -- it only keeps Rust from turning
        // the dead path into a bounds panic (STOP B carry item (nn)).
        return;
    }
    let keylen = key.len();
    let mut p = 0usize;

    while p < info.len() {
        // if (*info++ != '\\') break;  // error / end-of-string
        let c = info[p];
        p += 1;
        if c != b'\\' {
            break;
        }

        // !strncmp (info, key, keylen) && info[keylen] == '\\'
        // strncmp stops at either string's NUL, so a tail shorter than the key
        // can never match; the slice-length guard is that same test.
        if p + keylen <= info.len()
            && &info[p..p + keylen] == key
            && info.get(p + keylen).copied() == Some(b'\\')
        {
            // char *o = out, *e = out + outsize - 1;
            let e = out.len() - 1;
            let mut o = 0usize;

            // skip the key name
            p += keylen + 1;
            // this is the old value for the key. copy it to the result
            while p < info.len() && info[p] != b'\\' && o < e {
                out[o] = info[p] as c_char;
                o += 1;
                p += 1;
            }
            // `*o++ = 0`: o <= e == out.len() - 1, so this is in range
            out[o] = 0;

            // success!
            return;
        } else {
            // skip the key
            while p < info.len() && info[p] != b'\\' {
                p += 1;
            }

            // validate that its a value now
            if p >= info.len() {
                // C reads the NUL through `*info++`, which is != '\\'
                break; // error
            }
            let c = info[p];
            p += 1;
            if c != b'\\' {
                break; // error
            }
            // skip the value
            while p < info.len() && info[p] != b'\\' {
                p += 1;
            }
        }
    }
    out[0] = 0;
}

/// C `!strncmp ((char *)net_message.data + msg_readcount, lit, lit.len ())`.
///
/// # Safety
/// The caller must already have proved `msg_readcount + lit.len () <=
/// net_message.cursize`, which is what makes the `lit.len ()` bytes readable.
unsafe fn msg_tail_is(lit: &[u8]) -> bool {
    // SAFETY: caller contract
    unsafe {
        let raw = &raw const c::net_message;
        let p = (*raw).data.add(c::msg_readcount as usize);
        core::slice::from_raw_parts(p, lit.len()) == lit
    }
}

/// `strcpy (cname, name); [cname[14] = 0;] strcpy (name, "*"); strcat (name,
/// cname)` -- net_dgrm.c:1402-1406 (infoResponse, no truncation) and
/// :1487-1491 (CCREP_SERVER_INFO, `truncate14 = true`).
///
/// COMPAT / preserved defect, `truncate14 == false` only: `name` and `cname`
/// are both 64 bytes, and the infoResponse path fills `cname` from an
/// untruncated `name`, so `"*"` + up to 63 chars + NUL is up to 65 bytes and
/// the terminator lands on `map[0]` -- wiping the map name the C had just
/// parsed, and leaving `name` unterminated inside its own extent. The C does
/// this; reproducing it needs entry-base provenance, which `hc_field!`
/// supplies. The CCREP path's `cname[14] = 0` is what keeps that path inside
/// the field.
///
/// # Safety
/// `h` is a live `hostcache_t` from `host()`.
unsafe fn star_prefix_name(h: *mut HostCache, truncate14: bool) {
    // SAFETY: caller contract; see the COMPAT note above for the extent
    unsafe {
        let name = hc_field!(h, name);
        let cname = hc_field!(h, cname);
        strcpy(cname, name);
        if truncate14 {
            cname.add(14).write(0);
        }
        strcpy(name, c"*".as_ptr());
        strcat(name, cname);
    }
}

/// The "check for a name conflict" scan shared verbatim by both hostcache
/// paths (net_dgrm.c:1414-1440 and :1499-1525).
///
/// COMPAT / preserved defects, all transliterated:
///   * `i` is the `size_t` loop counter *and* the scratch for
///     `strlen (hostcache[n].name)`; assigning `(size_t)-1` and letting the
///     `i++` wrap to 0 is how the C restarts the scan.
///   * `hostcache[n].name[i - 1]` would index before the array for an empty
///     name. Both callers run `q_strlcpy (name, "UNNAMED", ...)` when the
///     name is empty first, so `strlen >= 1` here.
///   * `name[i]` / `name[i + 1]` are only written under `i < 15`, and
///     `name[i - 1]` only for `i = strlen (name) <= 63`; both are inside the
///     64-byte field.
///   * `name[i - 1]++` on a `char` can overflow; `wrapping_add` keeps C's
///     two's-complement result instead of a Rust debug panic. The `> '8'`
///     test is done at `c_char` so the platform's `char` signedness (and its
///     effect on high-bit name bytes) is preserved.
///
/// # Safety
/// Single-threaded host frame; `n < hostCacheCount <= HOSTCACHESIZE`.
unsafe fn resolve_name_conflict(n: usize) {
    // SAFETY: caller contract
    unsafe {
        let hn = host(n);
        let name_n = hc_field!(hn, name);
        let cname_n = hc_field!(hn, cname);

        let mut i: usize = 0;
        while i < c::hostCacheCount {
            if i == n {
                i = i.wrapping_add(1);
                continue;
            }
            let hi = host(i);
            if q_strcasecmp(cname_n, hc_field!(hi, cname)) == 0 {
                // this is a dupe.
                // hostCacheCount >= 1 here: the loop body only runs while
                // i < hostCacheCount, so the decrement cannot underflow.
                c::hostCacheCount -= 1;
                break;
            }
            if q_strcasecmp(name_n, hc_field!(hi, name)) == 0 {
                i = strlen(name_n);
                if i < 15 && name_n.add(i - 1).read() > b'8' as c_char {
                    name_n.add(i).write(b'0' as c_char);
                    name_n.add(i + 1).write(0);
                } else {
                    let v = name_n.add(i - 1).read();
                    name_n.add(i - 1).write(v.wrapping_add(1));
                }

                i = usize::MAX;
            }
            i = i.wrapping_add(1);
        }
    }
}

/// The shared tail of both hostcache paths: find `readaddr` in the cache, or
/// append. Returns `None` where the C `continue`s (:1392, :1477).
///
/// # Safety
/// Single-threaded host frame; the caller has already checked
/// `hostCacheCount != HOSTCACHESIZE`.
unsafe fn find_or_add_host(readaddr: &QSockAddr) -> Option<usize> {
    // SAFETY: caller contract
    unsafe {
        let ld = dfunc();
        // search the cache for this server
        let mut n = 0usize;
        while n < c::hostCacheCount {
            let mut a = *readaddr;
            let mut b = (*host(n)).addr;
            if ((*ld).addr_compare.expect("landriver AddrCompare"))(&mut a, &mut b) == 0 {
                break;
            }
            n += 1;
        }

        // is it already there?
        if n < c::hostCacheCount {
            // if (*hostcache[n].cname) continue;
            if (*host(n)).cname[0] != 0 {
                return None;
            }
        } else {
            // add it
            c::hostCacheCount += 1;
        }
        // n < HOSTCACHESIZE: the caller returned early on
        // hostCacheCount == HOSTCACHESIZE, so the increment above can only
        // reach HOSTCACHESIZE and n is the pre-increment value.
        Some(n)
    }
}

/// `_Datagram_SearchForHosts` (net_dgrm.c:1256)
///
/// # Safety
/// Single-threaded host frame; the landriver vtable is installed and
/// `net_message` is allocated.
unsafe fn datagram_search_for_hosts(xmit: QBoolean) -> QBoolean {
    // SAFETY: caller contract
    unsafe {
        // C leaves both uninitialized; GetSocketAddr/Read memset what they
        // fill, so zeroing only removes a Rust-side init requirement.
        let mut readaddr = QSockAddr::zeroed();
        let mut myaddr = QSockAddr::zeroed();
        let mut sentsomething: QBoolean = false;

        let ld = dfunc();
        ((*ld).get_socket_addr.expect("landriver GetSocketAddr"))((*ld).control_sock, &mut myaddr);

        if xmit {
            for e in hostlist().iter_mut() {
                e.requery = true;
            }

            build_server_info_request();
            {
                let raw = &raw const c::net_message;
                ((*ld).broadcast.expect("landriver Broadcast"))(
                    (*ld).control_sock,
                    (*raw).data,
                    (*raw).cursize,
                );
            }
            sz_clear_net_message();

            if slist_scope == SLIST_INTERNET {
                let mut m = 0usize;
                loop {
                    let master = NetDgrmOrch_Glue_MasterString(m);
                    if master.is_null() {
                        break;
                    }
                    if master.read() == 0 {
                        m += 1;
                        continue;
                    }
                    let mut masteraddr = QSockAddr::zeroed();
                    if ((*ld).get_addr_from_name.expect("landriver GetAddrFromName"))(
                        master,
                        &mut masteraddr,
                    ) >= 0
                    {
                        let mut prot = NetDgrmOrch_Glue_ProtocolName();
                        while prot.read() != 0 {
                            // send a request for each protocol
                            prot = c::COM_Parse(prot);
                            if prot.is_null() {
                                break;
                            }
                            let token = c::COM_ThreadToken();
                            if token.read() != 0 {
                                // Built in C so `va`'s VA_BUFFERLEN truncation
                                // and its \xff\xff\xff\xff prefix stay
                                // byte-identical (net_dgrm.c:1301-1304).
                                let ipv6 = c_int::from(
                                    masteraddr.qsa_family as c_int == NetDgrmOrch_Glue_AfInet6(),
                                );
                                let str_ = NetDgrmOrch_Glue_MasterQuery(
                                    ipv6,
                                    token,
                                    NET_PROTOCOL_VERSION as c_uint,
                                );
                                ((*ld).write.expect("landriver Write"))(
                                    (*ld).control_sock,
                                    str_.cast::<u8>().cast_mut(),
                                    strlen(str_) as c_int,
                                    &mut masteraddr,
                                );
                            }
                        }
                    }
                    m += 1;
                }
            }
            sentsomething = true;
        }

        loop {
            let ret = {
                let raw = &raw const c::net_message;
                ((*ld).read.expect("landriver Read"))(
                    (*ld).control_sock,
                    (*raw).data,
                    (*raw).maxsize,
                    &mut readaddr,
                )
            };
            if ret <= 0 {
                break;
            }
            if (ret as usize) < core::mem::size_of::<c_int>() {
                continue;
            }
            c::net_message.cursize = ret;

            // don't answer our own query
            // Note: this doesn't really work too well if we're multi-homed.
            // we should probably just refuse to respond to serverinfo requests
            // while we're scanning (chances are our server is going to die
            // anyway).
            {
                let mut a = readaddr;
                let mut b = myaddr;
                if ((*ld).addr_compare.expect("landriver AddrCompare"))(&mut a, &mut b) >= 0 {
                    continue;
                }
            }

            // is the cache full?
            if c::hostCacheCount == HOSTCACHESIZE {
                continue;
            }

            MSG_BeginReading();
            let control = {
                let raw = &raw const c::net_message;
                let mut hdr = [0u8; 4];
                core::ptr::copy_nonoverlapping((*raw).data, hdr.as_mut_ptr(), 4);
                // control = BigLong (*((int *)net_message.data))
                i32::from_be_bytes(hdr)
            };
            MSG_ReadLong();
            if control == -1 {
                let cursize = c::net_message.cursize;
                if c::msg_readcount + 19 <= cursize && msg_tail_is(b"getserversResponse") {
                    c::msg_readcount += 18;
                    loop {
                        // C declares `struct qsockaddr addr;` outside the
                        // loop uninitialised; every arm below assigns it
                        // before the read, so the scopes coincide.
                        let mut addr;
                        let tag = MSG_ReadByte();
                        if tag == b'\\' as c_int {
                            addr = QSockAddr::zeroed();
                            set_family(&mut addr, NetDgrmOrch_Glue_AfInet());
                            // sockaddr_in over qsockaddr: sin_port at
                            // qsa_data[0..2], sin_addr at qsa_data[2..6]
                            // (the offsets quake_net::udp uses).
                            for j in 0..4usize {
                                // MSG_ReadByte returns -1 at end of message;
                                // `(byte)(-1)` is 0xff in C and `as u8` here.
                                addr.qsa_data[2 + j] = MSG_ReadByte() as u8;
                            }
                            addr.qsa_data[0] = MSG_ReadByte() as u8;
                            addr.qsa_data[1] = MSG_ReadByte() as u8;
                            if addr.qsa_data[0] == 0 && addr.qsa_data[1] == 0 {
                                c::msg_badread = true;
                            }
                        } else if tag == b'/' as c_int {
                            addr = QSockAddr::zeroed();
                            set_family(&mut addr, NetDgrmOrch_Glue_AfInet6());
                            // sockaddr_in6: sin6_port at qsa_data[0..2],
                            // sin6_addr at qsa_data[6..22].
                            for j in 0..16usize {
                                addr.qsa_data[6 + j] = MSG_ReadByte() as u8;
                            }
                            addr.qsa_data[0] = MSG_ReadByte() as u8;
                            addr.qsa_data[1] = MSG_ReadByte() as u8;
                            if addr.qsa_data[0] == 0 && addr.qsa_data[1] == 0 {
                                c::msg_badread = true;
                            }
                        } else {
                            addr = QSockAddr::zeroed();
                            c::msg_badread = true;
                        }
                        if c::msg_badread {
                            break;
                        }
                        datagram_add_possible_host(&addr, true);
                        sentsomething = true;
                    }
                } else if c::msg_readcount + 13 <= cursize && msg_tail_is(b"infoResponse\n") {
                    // response from a dpp7 server (or possibly 15, no idea
                    // really)
                    let mut tmp = [0 as c_char; 1024];
                    let info: Vec<u8> = {
                        let s = MSG_ReadString();
                        let all = CStr::from_ptr(s).to_bytes();
                        // `MSG_ReadString () + 13`: the branch guard proved the
                        // next 13 bytes are "infoResponse\n", none of them NUL,
                        // so the string is always at least 13 bytes long.
                        all.get(13..).unwrap_or(&[]).to_vec()
                    };

                    let n = match find_or_add_host(&readaddr) {
                        Some(n) => n,
                        None => continue,
                    };
                    let h = host(n);

                    info_read_key(&info, b"hostname", &mut (*h).name);
                    if (*h).name[0] == 0 {
                        q_strlcpy(
                            hc_field!(h, name),
                            c"UNNAMED".as_ptr(),
                            core::mem::size_of_val(&(*h).name),
                        );
                    }
                    info_read_key(&info, b"mapname", &mut (*h).map);
                    info_read_key(&info, b"modname", &mut (*h).gamedir);

                    info_read_key(&info, b"clients", &mut tmp);
                    (*h).users = atoi(tmp.as_ptr());
                    info_read_key(&info, b"sv_maxclients", &mut tmp);
                    (*h).maxusers = atoi(tmp.as_ptr());
                    info_read_key(&info, b"protocol", &mut tmp);
                    if atoi(tmp.as_ptr()) != NET_PROTOCOL_VERSION as c_int {
                        // no cname[14] = 0 here, unlike the CCREP path -- see
                        // star_prefix_name's COMPAT note
                        star_prefix_name(h, false);
                    }
                    (*h).addr = readaddr;
                    (*h).driver = c::net_driverlevel;
                    (*h).ldriver = NET_LANDRIVERLEVEL;
                    {
                        let mut a = readaddr;
                        let s =
                            ((*ld).addr_to_string.expect("landriver AddrToString"))(&mut a, false);
                        q_strlcpy(hc_field!(h, cname), s, core::mem::size_of_val(&(*h).cname));
                    }

                    // check for a name conflict
                    resolve_name_conflict(n);
                }
                continue;
            }
            if (control & !(NETFLAG_LENGTH_MASK as i32)) != NETFLAG_CTL as i32 {
                continue;
            }
            if (control & NETFLAG_LENGTH_MASK as i32) != ret {
                continue;
            }

            if MSG_ReadByte() != CCREP_SERVER_INFO as c_int {
                continue;
            }

            MSG_ReadString();
            // dfunc.GetAddrFromName(MSG_ReadString(), &peeraddr);
            // (the C's commented-out "Server at %s claimed to be at %s" check
            // is not reproduced -- it is not compiled)

            let n = match find_or_add_host(&readaddr) {
                Some(n) => n,
                None => continue,
            };
            let h = host(n);

            q_strlcpy(
                hc_field!(h, name),
                MSG_ReadString(),
                core::mem::size_of_val(&(*h).name),
            );
            if (*h).name[0] == 0 {
                q_strlcpy(
                    hc_field!(h, name),
                    c"UNNAMED".as_ptr(),
                    core::mem::size_of_val(&(*h).name),
                );
            }
            q_strlcpy(
                hc_field!(h, map),
                MSG_ReadString(),
                core::mem::size_of_val(&(*h).map),
            );
            (*h).users = MSG_ReadByte();
            (*h).maxusers = MSG_ReadByte();
            if MSG_ReadByte() != NET_PROTOCOL_VERSION as c_int {
                star_prefix_name(h, true);
            }
            (*h).addr = readaddr;
            (*h).driver = c::net_driverlevel;
            (*h).ldriver = NET_LANDRIVERLEVEL;
            {
                let mut a = readaddr;
                let s = ((*ld).addr_to_string.expect("landriver AddrToString"))(&mut a, false);
                q_strlcpy(hc_field!(h, cname), s, core::mem::size_of_val(&(*h).cname));
            }

            // check for a name conflict
            resolve_name_conflict(n);
        }

        if !xmit {
            let mut n = 4; // should be time-based. meh.
            let level = NET_LANDRIVERLEVEL;
            let count = hostlist().len();
            for i in 0..count {
                // i < count == hostlist().len(), and nothing in this loop
                // pushes to the list, so both indexes are in range
                let e = hostlist()[i];
                if e.requery && e.driver == level {
                    hostlist()[i].requery = false;
                    datagram_send_server_query(&e.addr, e.master);
                    sentsomething = true;
                    n -= 1;
                    if n == 0 {
                        break;
                    }
                }
            }
        }
        sentsomething
    }
}

/// `Datagram_SearchForHosts` (net_dgrm.c:1547), wrapped by
/// `Quake/net_dgrm_orch_glue.c`.
///
/// No status channel: nothing reachable from here can raise. The subtree
/// touches only landriver vtable slots, the hostcache and the four pure
/// formatting seams in the glue (`_MasterString`, `_ProtocolName`,
/// `_MasterQuery`, `_AfInet`/`_AfInet6`), none of which reaches `Host_Error`,
/// so the slot keeps C's plain `qboolean` shape and the wrapper needs no
/// `Host_Reraise`.
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_search_for_hosts(xmit: QBoolean) -> QBoolean {
    // SAFETY: caller contract
    unsafe {
        let mut ret: QBoolean = false;
        NET_LANDRIVERLEVEL = 0;
        while NET_LANDRIVERLEVEL < net_numlandrivers {
            if c::hostCacheCount == HOSTCACHESIZE {
                break;
            }
            if (*landriver(NET_LANDRIVERLEVEL)).initialized {
                // `ret |= _Datagram_SearchForHosts (xmit)` over qboolean
                ret |= datagram_search_for_hosts(xmit);
            }
            NET_LANDRIVERLEVEL += 1;
        }
        ret
    }
}

// ############################ agent C2: connect handshake ###################

/// `MSG_ReadString ()` as owned bytes. `crate::net::MSG_ReadString` is the
/// live definition for the whole build and returns a NUL-terminated pointer
/// into its own 2048-byte static; the bytes are copied out immediately so the
/// next call cannot alias them.
fn read_string_bytes() -> Vec<u8> {
    // SAFETY: MSG_ReadString always returns a non-null NUL-terminated pointer
    unsafe {
        CStr::from_ptr(crate::net::MSG_ReadString())
            .to_bytes()
            .to_vec()
    }
}

/// Writes `m_return_reason` (`char [32]`, net_dgrm.c:62).
///
/// `_Datagram_Connect` uses two C forms: `strcpy` for the five literals
/// (longest is "Connect to Game failed", 22 bytes, so none can truncate) and
/// `q_strlcpy (..., sizeof (m_return_reason))` for the attacker-supplied
/// CCREP_REJECT text (net_dgrm.c:1718). Both are transliterated as the
/// truncating copy: 31 bytes + NUL. A hostile reject reason is up to 2047
/// bytes from MSG_ReadString, so the bound is load-bearing, not cosmetic.
///
/// # Safety
/// Single-threaded host frame.
unsafe fn set_m_return_reason(reason: &[u8]) {
    // SAFETY: caller contract; m_return_reason is a 32-byte C array, and n is
    // clamped to 31 so the NUL at [n] is the last byte at worst
    unsafe {
        let n = reason.len().min(31);
        let dst = (&raw mut m_return_reason).cast::<c_char>();
        for (i, &b) in reason.iter().take(n).enumerate() {
            *dst.add(i) = b as c_char;
        }
        *dst.add(n) = 0;
    }
}

/// `Con_Printf ("%s\n", reason)` (net_dgrm.c:1700 and friends)
fn print_reason(reason: &[u8]) {
    let mut line = reason.to_vec();
    line.push(b'\n');
    con_print_bytes(&line);
}

/// How `connect_handshake` left off.
enum ConnectFail {
    /// C's `goto ErrorReturn` -- free the qsocket, close the socket, bounce
    /// the menu, return NULL.
    Error,
    /// A `Host_Guard` caught a jump. C would have longjmp'd clean out of
    /// `_Datagram_Connect`, so NO cleanup runs (see the leak note in the
    /// notes file) and the status travels up to `Datagram_Connect`'s C
    /// wrapper, the ADR-009 raise frame.
    Raise(c_int),
}

/// net_dgrm.c:1806-1816 (`ErrorReturn2:` minus the qsocket free).
///
/// # Safety
/// `ld` is the landriver `newsock` was opened on; single-threaded host frame.
unsafe fn error_return2(ld: *mut NetLanDriver, newsock: SysSocket) {
    // SAFETY: caller contract
    unsafe {
        ((*ld).close_socket.expect("landriver Close_Socket"))(newsock);
        if m_return_onerror {
            key_dest = KEY_MENU;
            m_state = m_return_state;
            m_return_onerror = false;
        }
    }
}

/// net_dgrm.c:1582-1804 -- everything between `dfunc.Connect` and the
/// `return sock`, i.e. the part of `_Datagram_Connect` that owns `sock`.
///
/// # Safety
/// Single-threaded host frame; `ld` is the landriver `newsock` was opened on,
/// `sock` is a live pool qsocket and `serveraddr` a live `struct qsockaddr`.
unsafe fn connect_handshake(
    ld: *mut NetLanDriver,
    newsock: SysSocket,
    sock: *mut QSocket,
    serveraddr: *mut QSockAddr,
) -> Result<(), ConnectFail> {
    // SAFETY: caller contract; every C global touched here is host-thread-only
    unsafe {
        // connect to the host (:1582)
        if ((*ld).connect.expect("landriver Connect"))(newsock, serveraddr) == -1 {
            return Err(ConnectFail::Error);
        }

        (*sock).proquake_angle_hack = true;

        // send the connection request (:1588)
        con_safe_print("trying...\n");
        // ADR-009 raise site 1 of 3 (:1589). On a caught jump C never returns
        // here at all -- newsock and sock are leaked and m_return_onerror is
        // left set. Transliterated: no cleanup, status straight out.
        let g = NetDgrmOrch_Glue_UpdateScreen();
        if g != HOST_GUARD_OK {
            return Err(ConnectFail::Raise(g));
        }
        let mut start_time = c::net_time;

        let mut ret: c_int = 0;
        // C's `goto dpserveraccepted` (:1663), which skips the CCREP_* parse
        let mut dp_accepted = false;
        // C declares `readaddr` once for the whole function and leaves it
        // indeterminate; only a `ret > 0` Read reads it back, and that Read
        // always fills it, so the zero init is unobservable.
        let mut readaddr = QSockAddr::zeroed();

        // COMPAT: the retry count (3) and the 2.5s per-attempt window are
        // hardcoded in the C -- `net_connecttimeout` governs the *server*
        // browser paths, not this one. Loop shape, retry count and every
        // comparison operator are kept verbatim: an off-by-one here is
        // observable network behaviour under packet loss.
        'reps: for _ in 0..3 {
            let hack = (*sock).proquake_angle_hack;
            if hack {
                // :1603. Hoisted out of the sizebuf borrow below -- Con_* is
                // not a leaf (the M3 lesson in net.rs). Nothing else prints
                // between here and the write, so console order is unchanged.
                con_dwarning("Attempting to use ProQuake angle hack\n");
            }
            // :1594-1609. The pure quake_net writers, never the raising C
            // MSG_Write*: at most 19 bytes into a NET_MAXMESSAGE buffer, so
            // the `Err` arm (C's SZ_GetSpace Host_Error) is unreachable.
            with_net_message(|b| {
                b.clear();
                // save space for the header, filled in later
                let _ = msg::write_long(b, 0);
                let _ = msg::write_byte(b, c_int::from(CCREQ_CONNECT));
                let _ = msg::write_string(b, Some(b"QUAKE".as_slice()));
                let _ = msg::write_byte(b, c_int::from(NET_PROTOCOL_VERSION));
                if hack {
                    // Spike -- proquake compat
                    let _ = msg::write_byte(b, 1); // 'mod', 1=proquake
                    let _ = msg::write_byte(b, 34); // 'mod' version
                    let _ = msg::write_byte(b, 0); // flags
                    let _ = msg::write_long(b, 0); // password
                }
                // *((int *) net_message.data) = BigLong (...) -- BigLong is
                // the identity on a big-endian host, i.e. the word is written
                // big-endian either way
                let word = NETFLAG_CTL | (b.cursize as u32 & NETFLAG_LENGTH_MASK);
                if b.data.len() >= 4 {
                    b.data[0..4].copy_from_slice(&word.to_be_bytes());
                }
                // else: net_message is SZ_Alloc'ed to NET_MAXMESSAGE, so this
                // is unreachable; the guard only keeps the index panic-free
            });

            let nm = net_message();
            ((*ld).write.expect("landriver Write"))(newsock, (*nm).data, (*nm).cursize, serveraddr);
            sz_clear_net_message();

            // for dp compat (:1613-1618): DPGETCHALLENGE, sent every rep.
            // strlen() == 17; the leading four 0xff bytes are not NUL.
            let mut dp = *b"\xff\xff\xff\xffgetchallenge\n";
            ((*ld).write.expect("landriver Write"))(
                newsock,
                dp.as_mut_ptr(),
                dp.len() as c_int,
                serveraddr,
            );

            loop {
                // do-while body (:1620-1687). Each C `continue` lands on the
                // loop condition, so it maps to `break 'iter`.
                'iter: {
                    ret = ((*ld).read.expect("landriver Read"))(
                        newsock,
                        (*nm).data,
                        (*nm).maxsize,
                        &mut readaddr,
                    );
                    // if we got something, validate it
                    if ret > 0 {
                        // is it from the right place?
                        if ((*ld).addr_compare.expect("landriver AddrCompare"))(
                            &mut readaddr,
                            serveraddr,
                        ) != 0
                        {
                            con_safe_print("wrong reply address\n");
                            let mut line = b"Expected: ".to_vec();
                            line.extend_from_slice(&addr_to_string(ld, serveraddr));
                            line.extend_from_slice(b" | ");
                            line.extend_from_slice(CStr::from_ptr(str_addr(serveraddr)).to_bytes());
                            line.push(b'\n');
                            con_safe_print_bytes(&line);
                            let mut line = b"Received: ".to_vec();
                            line.extend_from_slice(&addr_to_string(ld, &mut readaddr));
                            line.extend_from_slice(b" | ");
                            line.extend_from_slice(
                                CStr::from_ptr(str_addr(&raw const readaddr)).to_bytes(),
                            );
                            line.push(b'\n');
                            con_safe_print_bytes(&line);
                            // ADR-009 raise site 2 of 3 (:1632)
                            let g = NetDgrmOrch_Glue_UpdateScreen();
                            if g != HOST_GUARD_OK {
                                return Err(ConnectFail::Raise(g));
                            }
                            ret = 0;
                            break 'iter;
                        }

                        if ret < core::mem::size_of::<c_int>() as c_int {
                            ret = 0;
                            break 'iter;
                        }

                        (*nm).cursize = ret;
                        crate::net::MSG_BeginReading();

                        // control = BigLong (*((int *) net_message.data)).
                        // In range: the `ret < sizeof (int)` test above and
                        // Read having filled `ret` bytes.
                        let d = (*nm).data;
                        let control = i32::from_be_bytes([*d, *d.add(1), *d.add(2), *d.add(3)]);
                        crate::net::MSG_ReadLong();
                        if control == -1 {
                            let s = read_string_bytes();
                            if s.starts_with(b"challenge ") {
                                // either a q2 or dp server...
                                // q_snprintf into `char buf[1024]`: the four
                                // %c 255s are literal 0xff bytes, and the
                                // result is truncated at 1023 + NUL, which
                                // `strlen (buf)` then measures. `s` can be
                                // 2047 bytes, so the truncation is reachable.
                                let mut buf: Vec<u8> = vec![255, 255, 255, 255];
                                buf.extend_from_slice(
                                    b"connect\\protocol\\darkplaces 3\\protocols\\RMQ FITZ DP7 NEHAHRABJP3 QUAKE\\challenge\\",
                                );
                                // in range: starts_with matched 10 bytes
                                buf.extend_from_slice(&s[10..]);
                                buf.truncate(1023);
                                ((*ld).write.expect("landriver Write"))(
                                    newsock,
                                    buf.as_mut_ptr(),
                                    buf.len() as c_int,
                                    serveraddr,
                                );
                            } else if s.as_slice() == b"accept" {
                                (*sock).addr = *serveraddr;
                                (*sock).proquake_angle_hack = false;
                                dp_accepted = true;
                                break 'reps; // goto dpserveraccepted
                            }
                            // the `reject` branch is commented out in the C
                            // (:1665-1671); left out here too

                            ret = 0;
                            break 'iter;
                        }
                        if (control & !(NETFLAG_LENGTH_MASK as c_int)) != NETFLAG_CTL as c_int {
                            ret = 0;
                            break 'iter;
                        }
                        if (control & NETFLAG_LENGTH_MASK as c_int) != ret {
                            ret = 0;
                            break 'iter;
                        }
                    }
                }
                // `while (ret == 0 && (SetNetTime () - start_time) < 2.5)` --
                // the && short-circuits, so SetNetTime (which republishes
                // net_time) does NOT run once ret is non-zero.
                if !(ret == 0 && (c::SetNetTime() - start_time) < 2.5) {
                    break;
                }
            }

            if ret != 0 {
                break 'reps;
            }

            con_safe_print("still trying...\n");
            // ADR-009 raise site 3 of 3 (:1693). Fires on the third rep too:
            // C prints and updates the screen before the loop counter retires
            // it.
            let g = NetDgrmOrch_Glue_UpdateScreen();
            if g != HOST_GUARD_OK {
                return Err(ConnectFail::Raise(g));
            }
            start_time = c::SetNetTime();
        }

        if !dp_accepted {
            if ret == 0 {
                let reason = b"No Response";
                print_reason(reason);
                set_m_return_reason(reason);
                return Err(ConnectFail::Error);
            }

            if ret == -1 {
                let reason = b"Network Error";
                print_reason(reason);
                set_m_return_reason(reason);
                return Err(ConnectFail::Error);
            }

            ret = crate::net::MSG_ReadByte();
            if ret == c_int::from(CCREP_REJECT) {
                let reason = read_string_bytes();
                print_reason(&reason);
                set_m_return_reason(&reason);
                return Err(ConnectFail::Error);
            }

            if ret == c_int::from(CCREP_ACCEPT) {
                (*sock).addr = *serveraddr;
                let port = crate::net::MSG_ReadLong();
                // spike -- don't change the remote port if the server doesn't
                // want us to
                if port != 0 {
                    ((*ld).set_socket_port.expect("landriver SetSocketPort"))(
                        &raw mut (*sock).addr,
                        port,
                    );
                }
            } else {
                let reason = b"Bad Response";
                print_reason(reason);
                set_m_return_reason(reason);
                return Err(ConnectFail::Error);
            }

            if (*sock).proquake_angle_hack {
                let nm = net_message();
                // `byte x = (msg_readcount < cursize) ? MSG_ReadByte () : 0;`
                // -- the guard keeps MSG_ReadByte's -1 underrun out, so the
                // int-to-byte narrowing never sees a negative value.
                let m = if c::msg_readcount < (*nm).cursize {
                    crate::net::MSG_ReadByte() as u8
                } else {
                    0
                };
                let ver = if c::msg_readcount < (*nm).cursize {
                    crate::net::MSG_ReadByte() as u8
                } else {
                    0
                };
                let flags = if c::msg_readcount < (*nm).cursize {
                    crate::net::MSG_ReadByte() as u8
                } else {
                    0
                };
                let _ = ver; // (void) ver;

                if m == 1
                /* MOD_PROQUAKE */
                {
                    if flags & 1 != 0
                    /* CHEATFREE */
                    {
                        let reason = b"Server is incompatible";
                        print_reason(reason);
                        set_m_return_reason(reason);
                        return Err(ConnectFail::Error);
                    }
                    (*sock).proquake_angle_hack = true;
                } else {
                    (*sock).proquake_angle_hack = false;
                }
            }
        }

        // dpserveraccepted: (:1761)
        let get_name = (*ld).get_name_from_addr.expect("landriver GetNameFromAddr");
        get_name(serveraddr, (&raw mut (*sock).trueaddress).cast::<c_char>());
        get_name(
            serveraddr,
            (&raw mut (*sock).maskedaddress).cast::<c_char>(),
        );

        con_print("Connection accepted\n");
        (*sock).last_message_time = c::SetNetTime();

        // switch the connection to the specified address (:1770)
        if ((*ld).connect.expect("landriver Connect"))(newsock, &raw mut (*sock).addr) == -1 {
            let reason = b"Connect to Game failed";
            print_reason(reason);
            set_m_return_reason(reason);
            return Err(ConnectFail::Error);
        }

        Ok(())
    }
}

/// `_Datagram_Connect` (net_dgrm.c:1560).
///
/// ADR-009: returns a `Host_Guard` status and hands the `qsocket_t *` back
/// through `out`. All three `SCR_UpdateScreen (false)` sites (`:1589`,
/// `:1632`, `:1693`) leave `*out` NULL, run no cleanup (matching the C
/// longjmp) and return the status for `Datagram_Connect`'s C wrapper to
/// re-raise.
///
/// # Safety
/// Single-threaded host frame; `serveraddr` is a live `struct qsockaddr` and
/// `out` a writable `qsocket_t *`.
unsafe fn datagram_connect_one(serveraddr: *mut QSockAddr, out: *mut *mut QSocket) -> c_int {
    // SAFETY: caller contract
    unsafe {
        *out = core::ptr::null_mut();

        let ld = dfunc();
        let newsock = ((*ld).open_socket.expect("landriver Open_Socket"))(0);
        if newsock == INVALID_SOCKET {
            return HOST_GUARD_OK;
        }

        let sock = c::NET_NewQSocket().cast::<QSocket>();
        if sock.is_null() {
            // goto ErrorReturn2 (:1577)
            error_return2(ld, newsock);
            return HOST_GUARD_OK;
        }
        (*sock).socket = newsock;
        (*sock).landriver = NET_LANDRIVERLEVEL;

        match connect_handshake(ld, newsock, sock, serveraddr) {
            Ok(()) => {
                m_return_onerror_clear();
                *out = sock;
                HOST_GUARD_OK
            }
            Err(ConnectFail::Error) => {
                // ErrorReturn: (:1806)
                c::NET_FreeQSocket(sock.cast());
                error_return2(ld, newsock);
                HOST_GUARD_OK
            }
            // COMPAT (net_dgrm.c, preserved defect): the C's Host_Error
            // longjmp abandons this frame, so neither NET_FreeQSocket nor
            // Close_Socket runs and m_return_onerror keeps whatever the menu
            // set. The qsocket and the system socket leak. Reproduced.
            Err(ConnectFail::Raise(g)) => g,
        }
    }
}

/// `m_return_onerror = false;` (:1803)
fn m_return_onerror_clear() {
    // SAFETY: single-threaded host frame
    unsafe {
        m_return_onerror = false;
    }
}

/// `Datagram_Connect` (net_dgrm.c:1819), the `net_drivers[].Connect` slot.
/// `Quake/net_dgrm_orch_glue.c` owns the raise frame.
///
/// # Safety
/// Single-threaded host frame; `out` is a writable `qsocket_t *`. `host` is
/// the caller's NUL-terminated string (or NULL, as in C).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_connect(
    host: *const c_char,
    out: *mut *mut QSocket,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        *out = core::ptr::null_mut();

        let mut ret: *mut QSocket = core::ptr::null_mut();
        let mut resolved = false;
        // C leaves `addr` indeterminate; GetAddrFromName fills it before any
        // read, so zeroing it is unobservable
        let mut addr = QSockAddr::zeroed();

        let host = strip_port(host);

        NET_LANDRIVERLEVEL = 0;
        while NET_LANDRIVERLEVEL < net_numlandrivers {
            let ld = dfunc();
            if (*ld).initialized {
                // see if we can resolve the host name
                // Spike -- moved name resolution to here to avoid extraneous
                // 'could not resolves' when using other address families
                if ((*ld).get_addr_from_name.expect("landriver GetAddrFromName"))(host, &mut addr)
                    != -1
                {
                    resolved = true;
                    let mut sockout: *mut QSocket = core::ptr::null_mut();
                    let g = datagram_connect_one(&mut addr, &mut sockout);
                    if g != HOST_GUARD_OK {
                        // C's longjmp leaves this frame too: the loop is
                        // abandoned mid-iteration (net_landriverlevel keeps
                        // its current value, as in C) and nothing more is
                        // printed.
                        return g;
                    }
                    ret = sockout;
                    if !ret.is_null() {
                        break;
                    }
                }
            }
            NET_LANDRIVERLEVEL += 1;
        }
        if !resolved {
            let mut line = b"Could not resolve ".to_vec();
            // COMPAT: C passes `host` straight to Con_SafePrintf's %s. A NULL
            // host is UB there (the CRTs print "(null)"); Rust cannot deref
            // it, so the same text is substituted. Unreachable from the two
            // real callers (Host_Connect_f, NET_Connect), both of which pass
            // a Cmd_Argv/cls string.
            if host.is_null() {
                line.extend_from_slice(b"(null)");
            } else {
                line.extend_from_slice(CStr::from_ptr(host).to_bytes());
            }
            line.push(b'\n');
            con_safe_print_bytes(&line);
        }
        *out = ret;
        HOST_GUARD_OK
    }
}

/*
Spike: added this to list more than one ipv4 address (many people are still multi-homed)
*/
/// `Datagram_QueryAddresses` (net_dgrm.c:1848), the
/// `net_drivers[].QueryAddresses` slot. Nothing below it can raise -- the two
/// live implementations are `rust_udp4_GetAddresses` / `rust_udp6_GetAddresses`
/// (net_bsd.c:80, :102 and net_win.c) and the loop driver leaves the slot NULL
/// -- so the glue wrapper is a plain pass-through with no `Host_Reraise`.
///
/// # Safety
/// Single-threaded host frame; `addresses` covers `maxaddresses` entries.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_query_addresses(
    addresses: *mut QHostAddr,
    maxaddresses: c_int,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let mut result: c_int = 0;
        NET_LANDRIVERLEVEL = 0;
        while NET_LANDRIVERLEVEL < net_numlandrivers {
            let ld = dfunc();
            if !(*ld).initialized {
                NET_LANDRIVERLEVEL += 1;
                continue;
            }
            if result == maxaddresses {
                break;
            }
            if let Some(q) = (*ld).query_addresses {
                // COMPAT (preserved defect): C tests `result == maxaddresses`,
                // not `>=`, and never clamps what a driver returns. A driver
                // that reports more entries than it was asked for walks
                // `addresses` past the caller's array and passes the next
                // driver a negative `maxaddresses - result`. Transliterated
                // with C's pointer arithmetic.
                result += q(addresses.offset(result as isize), maxaddresses - result);
            }
            NET_LANDRIVERLEVEL += 1;
        }
        result
    }
}

// ############################ agent D: test / test2 #########################

extern "C" {
    /// `net_defs.h:215` -- `extern const int net_numlandrivers;`. Declared
    /// locally rather than through quake-c-sys, matching net_main.rs's
    /// identical local declaration of the driver-count sibling
    /// `net_numdrivers`.
    static net_numlandrivers: c_int;
}

fn con_print(text: &str) {
    let mut b = text.as_bytes().to_vec();
    b.push(0);
    // SAFETY: b is NUL-terminated
    unsafe {
        c::Con_Printf(c"%s".as_ptr(), b.as_ptr());
    }
}

/// C's `%-W.Ws` on an owned byte buffer -- the byte-slice equivalent of
/// net_main.rs's `pad_trunc` (which operates on a `&[c_char]` fixed C
/// field; mine takes a `Vec<u8>` from `MsgReader::read_string()`).
fn pad_trunc_bytes(src: &[u8], width: usize) -> Vec<u8> {
    let mut b = src.to_vec();
    b.truncate(width);
    while b.len() < width {
        b.push(b' ');
    }
    b
}

/// `quakedef.h:214` -- `#define MAX_SCOREBOARD 16`, redeclared locally
/// matching the existing per-file precedent (cl_parse.rs, host.rs).
const MAX_SCOREBOARD: usize = 16;

/// `net_dgrm.c:305-310`
static mut TEST_IN_PROGRESS: bool = false;
static mut TEST_POLL_COUNT: c_int = 0;
static mut TEST_DRIVER: c_int = 0;
static mut TEST_SOCKET: SysSocket = 0;
static mut TEST_POLL_PROCEDURE: PollProcedure = PollProcedure {
    next: core::ptr::null_mut(),
    next_time: 0.0,
    procedure: Some(quake_rs_dgrm_test_poll),
    arg: core::ptr::null_mut(),
};

/// `net_dgrm.c:436-443`
static mut TEST2_IN_PROGRESS: bool = false;
static mut TEST2_DRIVER: c_int = 0;
static mut TEST2_SOCKET: SysSocket = 0;
static mut TEST2_POLL_PROCEDURE: PollProcedure = PollProcedure {
    next: core::ptr::null_mut(),
    next_time: 0.0,
    procedure: Some(quake_rs_dgrm_test2_poll),
    arg: core::ptr::null_mut(),
};

/// `Test_Poll` (`net_dgrm.c:312-367`)
///
/// # Safety
/// Called only as a `PollProcedure.procedure` callback from the (still-C)
/// `NET_Poll` scheduler; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_test_poll(_unused: *mut c_void) {
    // SAFETY: caller contract
    unsafe {
        NET_LANDRIVERLEVEL = TEST_DRIVER;
        loop {
            let nm = &raw mut c::net_message;
            let maxsize = (*nm).maxsize;
            let ld = landriver(NET_LANDRIVERLEVEL);
            let mut clientaddr = QSockAddr::zeroed();
            let len = {
                let buf = core::slice::from_raw_parts_mut((*nm).data, maxsize as usize);
                ((*ld).read.expect("landriver Read"))(
                    TEST_SOCKET,
                    buf.as_mut_ptr(),
                    maxsize,
                    &mut clientaddr,
                )
            };
            // C: `len < (int)sizeof(int)` -- a signed comparison. Comparing
            // via `len < 4` (not `(len as usize) < 4`) matters: a negative
            // `len` (landriver read error) must break here, and casting a
            // negative i32 to usize first would wrap past the check.
            if len < core::mem::size_of::<c_int>() as i32 {
                break;
            }
            (*nm).cursize = len;

            // control header: BigLong() over the raw leading 4 bytes. The
            // host is always little-endian (no Swap_Init call ever
            // reassigns BigLong away from LongSwap -- rust/quake-net/src/
            // dgrm.rs precedent), so BigLong == a byte-swap == interpreting
            // the header as big-endian.
            let data = core::slice::from_raw_parts((*nm).data, maxsize as usize);
            let control = i32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            let mut reader = MsgReader::begin(data, len);
            reader.read_long(); // MSG_ReadLong(): return discarded in C too,
                                // only the readcount += 4 side effect matters

            if control == -1 {
                break;
            }
            if (control as u32 & !NETFLAG_LENGTH_MASK) != NETFLAG_CTL {
                break;
            }
            if (control as u32 & NETFLAG_LENGTH_MASK) as i32 != len {
                break;
            }

            if reader.read_byte() != CCREP_PLAYER_INFO as i32 {
                // COMPAT: Sys_Error aborts the process outright (no
                // longjmp) -- called as a plain extern per the contract's
                // ADR-009 raise inventory (net_dgrm.c:344 is this call).
                c::Sys_Error(c"Unexpected repsonse to Player Info request\n".as_ptr());
            }
            reader.read_byte(); // playerNumber -- read and discarded, as C

            // COMPAT: C's `strcpy (name, MSG_ReadString ())` into
            // `char name[32]` (and `address[64]` below) is a real,
            // remotely-triggerable stack-buffer overflow (MSG_ReadString
            // caps at 2047 bytes) that cannot be reproduced without
            // recreating the same memory-safety bug, and an implicit
            // bounds panic is forbidden under panic=abort. Divergence: an
            // owned Vec<u8> holds the full string and Con_Printf prints all
            // of it, instead of C's truncated-or-corrupted stack contents.
            let name = reader.read_string();
            let colors = reader.read_long();
            let frags = reader.read_long();
            let connect_time = reader.read_long();
            let address = reader.read_string(); // COMPAT: same overflow class

            let mut line: Vec<u8> = Vec::new();
            line.extend_from_slice(&name);
            line.extend_from_slice(b"\n  frags:");
            line.extend_from_slice(format!("{:3}", frags).as_bytes());
            line.extend_from_slice(b"  colors:");
            line.extend_from_slice(format!("{}", colors >> 4).as_bytes());
            line.push(b' ');
            line.extend_from_slice(format!("{}", colors & 0x0f).as_bytes());
            line.extend_from_slice(b"  time:");
            line.extend_from_slice(format!("{}", connect_time / 60).as_bytes());
            line.extend_from_slice(b"\n  ");
            line.extend_from_slice(&address);
            line.push(b'\n');
            con_print_bytes(&line);
        }
        TEST_POLL_COUNT -= 1;
        if TEST_POLL_COUNT != 0 {
            SchedulePollProcedure((&raw mut TEST_POLL_PROCEDURE).cast(), 0.1);
        } else {
            let ld = landriver(NET_LANDRIVERLEVEL);
            ((*ld).close_socket.expect("landriver Close_Socket"))(TEST_SOCKET);
            TEST_IN_PROGRESS = false;
        }
    }
}

/// `Test_f` (`net_dgrm.c:368-444`)
///
/// # Safety
/// Command context (registered on `"test"` by `Datagram_Init`, owned by
/// agent A); single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_test_f() {
    // SAFETY: caller contract
    unsafe {
        if TEST_IN_PROGRESS {
            return;
        }

        // `host = Strip_Port (Cmd_Argv (1))`. Pointer-valued exactly as C:
        // either the caller's own `cmd_argv` storage or `Strip_Port`'s
        // `noport` static, both live for the whole command.
        let hostname = strip_port(c::Cmd_Argv(1));

        let mut maxusers: usize = MAX_SCOREBOARD;
        let mut sendaddr = QSockAddr::zeroed();
        let mut found = false;

        // COMPAT: C's `if (host && hostCacheCount)` tests a pointer for
        // NULL, not the string for emptiness. Cmd_Argv/Strip_Port never
        // return NULL (see notes), so this is always just
        // `if (hostCacheCount)`.
        if c::hostCacheCount > 0 {
            for n in 0..c::hostCacheCount {
                let h = &*host(n);
                if q_strcasecmp(hostname, h.name.as_ptr()) == 0 {
                    if h.driver != MY_DRIVER_LEVEL {
                        continue;
                    }
                    NET_LANDRIVERLEVEL = h.ldriver;
                    // COMPAT: C's implicit int -> size_t conversion of a
                    // negative `maxusers` (attacker-controlled hostcache
                    // entry) wraps to a near-usize::MAX loop bound below --
                    // a real DoS in the C original (not a memory-safety
                    // bug: nothing is indexed by it). Preserved as-is.
                    maxusers = h.maxusers as usize;
                    sendaddr = h.addr;
                    found = true;
                    break;
                }
            }
        }

        if !found {
            NET_LANDRIVERLEVEL = 0;
            while NET_LANDRIVERLEVEL < net_numlandrivers {
                let ld = landriver(NET_LANDRIVERLEVEL);
                if (*ld).initialized
                    && ((*ld).get_addr_from_name.expect("landriver GetAddrFromName"))(
                        hostname,
                        &mut sendaddr,
                    ) != -1
                {
                    break;
                }
                NET_LANDRIVERLEVEL += 1;
            }
            if NET_LANDRIVERLEVEL == net_numlandrivers {
                let mut msgtext = b"Could not resolve ".to_vec();
                msgtext.extend_from_slice(CStr::from_ptr(hostname).to_bytes());
                msgtext.push(b'\n');
                con_print_bytes(&msgtext);
                return;
            }
        }

        let ld = landriver(NET_LANDRIVERLEVEL);
        TEST_SOCKET = ((*ld).open_socket.expect("landriver Open_Socket"))(0);
        if TEST_SOCKET == INVALID_SOCKET {
            return;
        }
        TEST_IN_PROGRESS = true;
        TEST_POLL_COUNT = 20;
        TEST_DRIVER = NET_LANDRIVERLEVEL;

        for n in 0..maxusers {
            let r = with_net_message(|sb| {
                sb.clear();
                msg::write_long(sb, 0)?;
                msg::write_byte(sb, CCREQ_PLAYER_INFO as i32)?;
                // COMPAT: MSG_WriteByte truncates `n` (size_t) to a byte,
                // same as C -- reachable for n > 255 via the maxusers DoS
                // defect above.
                msg::write_byte(sb, n as i32)?;
                let header =
                    (NETFLAG_CTL | (sb.cursize as u32 & NETFLAG_LENGTH_MASK)).to_be_bytes();
                sb.data[0..4].copy_from_slice(&header);
                Ok::<(), WireError>(())
            });
            if r.is_err() {
                // Unreachable, and provably so: `sb.clear()` above puts
                // cursize at 0 and the three writes total six bytes into
                // net_message's NET_MAXMESSAGE (64000) allocation, so the C's
                // SZ_GetSpace Host_Error has no way to fire. `Test_f` is a
                // void Cmd_AddCommand handler with no status channel, so
                // there is nowhere to raise to even if it could.
                continue;
            }
            let nm = &raw mut c::net_message;
            let ld = landriver(NET_LANDRIVERLEVEL);
            ((*ld).write.expect("landriver Write"))(
                TEST_SOCKET,
                (*nm).data,
                (*nm).cursize,
                &mut sendaddr,
            );
        }
        sz_clear_net_message();
        SchedulePollProcedure((&raw mut TEST_POLL_PROCEDURE).cast(), 0.1);
    }
}

/// `Test2_Poll` (`net_dgrm.c:445-502`)
///
/// # Safety
/// Called only as a `PollProcedure.procedure` callback from the (still-C)
/// `NET_Poll` scheduler; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_test2_poll(_unused: *mut c_void) {
    // SAFETY: caller contract
    unsafe {
        NET_LANDRIVERLEVEL = TEST2_DRIVER;
        // C's `name[0] = 0` before the first read is a dead store: every
        // exit path either overwrites `name` from the wire before it is
        // read, or never reads `name` at all. Omitted.

        let nm = &raw mut c::net_message;
        let maxsize = (*nm).maxsize;
        let ld = landriver(NET_LANDRIVERLEVEL);
        let mut clientaddr = QSockAddr::zeroed();
        let len = {
            let buf = core::slice::from_raw_parts_mut((*nm).data, maxsize as usize);
            ((*ld).read.expect("landriver Read"))(
                TEST2_SOCKET,
                buf.as_mut_ptr(),
                maxsize,
                &mut clientaddr,
            )
        };
        if len < core::mem::size_of::<c_int>() as i32 {
            SchedulePollProcedure((&raw mut TEST2_POLL_PROCEDURE).cast(), 0.05);
            return;
        }
        (*nm).cursize = len;

        let data = core::slice::from_raw_parts((*nm).data, maxsize as usize);
        let control = i32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let mut reader = MsgReader::begin(data, len);
        reader.read_long(); // discard; advances readcount to 4, as C

        let mut error = control == -1;
        if !error && (control as u32 & !NETFLAG_LENGTH_MASK) != NETFLAG_CTL {
            error = true;
        }
        if !error && (control as u32 & NETFLAG_LENGTH_MASK) as i32 != len {
            error = true;
        }
        if !error && reader.read_byte() != CCREP_RULE_INFO as i32 {
            error = true;
        }

        if error {
            con_print("Unexpected repsonse to Rule Info request\n");
            close_test2();
            return;
        }

        // COMPAT: same strcpy-into-fixed-array overflow class as
        // Test_Poll, here against char name[256]/value[256]; see notes.
        let name = reader.read_string();
        if name.is_empty() {
            close_test2();
            return;
        }
        let value = reader.read_string();

        let mut line = pad_trunc_bytes(&name, 16);
        line.push(b' ');
        line.push(b' ');
        line.extend_from_slice(&pad_trunc_bytes(&value, 16));
        line.push(b'\n');
        con_print_bytes(&line);

        let sent = with_net_message(|sb| {
            sb.clear();
            msg::write_long(sb, 0)?;
            msg::write_byte(sb, CCREQ_RULE_INFO as i32)?;
            msg::write_string(sb, Some(&name))?;
            let header = (NETFLAG_CTL | (sb.cursize as u32 & NETFLAG_LENGTH_MASK)).to_be_bytes();
            sb.data[0..4].copy_from_slice(&header);
            Ok::<(), WireError>(())
        });
        if sent.is_ok() {
            let nm = &raw mut c::net_message;
            let ld = landriver(NET_LANDRIVERLEVEL);
            ((*ld).write.expect("landriver Write"))(
                TEST2_SOCKET,
                (*nm).data,
                (*nm).cursize,
                &mut clientaddr,
            );
            sz_clear_net_message();
        }

        SchedulePollProcedure((&raw mut TEST2_POLL_PROCEDURE).cast(), 0.05);
    }
}

/// Shared close+flag-reset tail of `Test2_Poll`'s `Error:`/`Done:` labels.
///
/// # Safety
/// Same host-frame contract as `quake_rs_dgrm_test2_poll`.
unsafe fn close_test2() {
    // SAFETY: caller contract
    unsafe {
        let ld = landriver(NET_LANDRIVERLEVEL);
        ((*ld).close_socket.expect("landriver Close_Socket"))(TEST2_SOCKET);
        TEST2_IN_PROGRESS = false;
    }
}

/// `Test2_f` (`net_dgrm.c:503-565`)
///
/// # Safety
/// Command context (registered on `"test2"` by `Datagram_Init`, owned by
/// agent A); single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_dgrm_test2_f() {
    // SAFETY: caller contract
    unsafe {
        if TEST2_IN_PROGRESS {
            return;
        }

        // `host = Strip_Port (Cmd_Argv (1))`. Pointer-valued exactly as C:
        // either the caller's own `cmd_argv` storage or `Strip_Port`'s
        // `noport` static, both live for the whole command.
        let hostname = strip_port(c::Cmd_Argv(1));

        let mut sendaddr = QSockAddr::zeroed();
        let mut found = false;

        // COMPAT: same `host &&` always-true simplification as Test_f
        if c::hostCacheCount > 0 {
            for n in 0..c::hostCacheCount {
                let h = &*host(n);
                if q_strcasecmp(hostname, h.name.as_ptr()) == 0 {
                    if h.driver != MY_DRIVER_LEVEL {
                        continue;
                    }
                    NET_LANDRIVERLEVEL = h.ldriver;
                    sendaddr = h.addr;
                    found = true;
                    break;
                }
            }
        }

        if !found {
            NET_LANDRIVERLEVEL = 0;
            while NET_LANDRIVERLEVEL < net_numlandrivers {
                let ld = landriver(NET_LANDRIVERLEVEL);
                if (*ld).initialized
                    && ((*ld).get_addr_from_name.expect("landriver GetAddrFromName"))(
                        hostname,
                        &mut sendaddr,
                    ) != -1
                {
                    break;
                }
                NET_LANDRIVERLEVEL += 1;
            }
            if NET_LANDRIVERLEVEL == net_numlandrivers {
                let mut msgtext = b"Could not resolve ".to_vec();
                msgtext.extend_from_slice(CStr::from_ptr(hostname).to_bytes());
                msgtext.push(b'\n');
                con_print_bytes(&msgtext);
                return;
            }
        }

        let ld = landriver(NET_LANDRIVERLEVEL);
        TEST2_SOCKET = ((*ld).open_socket.expect("landriver Open_Socket"))(0);
        if TEST2_SOCKET == INVALID_SOCKET {
            return;
        }
        TEST2_IN_PROGRESS = true;
        TEST2_DRIVER = NET_LANDRIVERLEVEL;

        let sent = with_net_message(|sb| {
            sb.clear();
            msg::write_long(sb, 0)?;
            msg::write_byte(sb, CCREQ_RULE_INFO as i32)?;
            msg::write_string(sb, Some(b""))?;
            let header = (NETFLAG_CTL | (sb.cursize as u32 & NETFLAG_LENGTH_MASK)).to_be_bytes();
            sb.data[0..4].copy_from_slice(&header);
            Ok::<(), WireError>(())
        });
        if sent.is_ok() {
            let nm = &raw mut c::net_message;
            let ld = landriver(NET_LANDRIVERLEVEL);
            ((*ld).write.expect("landriver Write"))(
                TEST2_SOCKET,
                (*nm).data,
                (*nm).cursize,
                &mut sendaddr,
            );
        }
        sz_clear_net_message();
        SchedulePollProcedure((&raw mut TEST2_POLL_PROCEDURE).cast(), 0.05);
    }
}
