//! Phase 5 M6: the datagram reliable/unreliable wire layer, transliterated
//! from `Quake/net_dgrm_rel.c` (split out of net_dgrm.c at M6).
//!
//! Both RX paths are ported verbatim and separately -- `process_packet`
//! (the server path, called from Datagram_GetAnyMessage) and `get_message`
//! (the client path) duplicate their sequencing logic in C and their small
//! divergences (ACK destination address, oversize handling, return codes)
//! are load-bearing, so they stay duplicated here.
//!
//! System IO (`sfunc.Read/Write/AddrCompare/AddrToString`) and console
//! diagnostics go through the [`NetSys`] trait: the engine (quake-capi, M7)
//! implements it over the C `net_landrivers[]` vtable, the differential
//! tests over a deterministic mock shared with the c_ref oracle.
//!
//! The C file's `packetBuffer` static is shared scratch between the send
//! and receive paths, and its stale contents are *observable*: a packet
//! whose wire-header length exceeds the bytes actually received makes C
//! copy stale scratch bytes into net_message / receiveMessage. Every
//! function here therefore takes the same persistent `pkt` scratch slice
//! (the engine passes the C static itself), reproducing that behavior.
//!
//! Error posture (ADR-009): the only Host_Error-capable C path is
//! get_message's unreliable branch (its SZ_Write has no maxsize pre-check),
//! surfaced as [`GET_MESSAGE_NET_MESSAGE_OVERFLOW`] for the M7 C glue frame
//! to re-raise; every other net_message write is pre-checked. The
//! DEBUG-only `Sys_Error`s in the send paths surface as [`DgrmError`].

use quake_types::net::{
    QSockAddr, QSocket, SysSocket, NETFLAG_ACK, NETFLAG_CTL, NETFLAG_DATA, NETFLAG_EOM,
    NETFLAG_LENGTH_MASK, NETFLAG_UNRELIABLE, NET_HEADERSIZE, NET_MAXMESSAGE,
};

/// `sizeof packetBuffer` (dgrm_packet_t): 8-byte header + MAX_DATAGRAM data
pub const PACKET_BUFFER_SIZE: usize = quake_types::net::NET_DATAGRAMSIZE;

/// System-IO boundary standing in for `sfunc` + the console.
pub trait NetSys {
    /// `sfunc.Read`: fill `buf`, report (length, source address).
    /// length 0 = nothing pending, -1 = error (address then unspecified).
    fn read(&mut self, socket: SysSocket, buf: &mut [u8]) -> (i32, QSockAddr);
    /// `sfunc.Write`: returns -1 on error
    fn write(&mut self, socket: SysSocket, buf: &[u8], addr: &QSockAddr) -> i32;
    /// `sfunc.AddrCompare`
    fn addr_compare(&mut self, a: &QSockAddr, b: &QSockAddr) -> i32;
    /// `sfunc.AddrToString` (masked=false at every rel-layer call site)
    fn addr_to_string(&mut self, addr: &QSockAddr) -> String;
    /// `Con_Printf`
    fn print(&mut self, msg: &str);
    /// `Con_DPrintf`
    fn dprint(&mut self, msg: &str);
}

/// The net_dgrm_rel.c statistic counters (C-owned globals in the engine;
/// the M7 shim marshals them around these calls).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct DgrmCounters {
    pub packets_sent: i32,
    pub packets_resent: i32,
    pub packets_received: i32,
    pub received_duplicate_count: i32,
    pub short_packet_count: i32,
    pub dropped_datagrams: i32,
}

/// The ambient engine state the rel layer reads/writes besides the qsocket:
/// `net_time`, the counters, the net_main message counters, and the
/// C-owned `net_message` sizebuf storage.
pub struct DgrmGlobals<'a> {
    pub net_time: f64,
    pub counters: &'a mut DgrmCounters,
    /// net_main.c `messagesReceived`
    pub messages_received: &'a mut i32,
    /// net_main.c `unreliableMessagesReceived`
    pub unreliable_messages_received: &'a mut i32,
    /// net_message.data (full allocation)
    pub net_message: &'a mut [u8],
    /// net_message.cursize
    pub net_message_cursize: &'a mut i32,
    /// net_message.maxsize
    pub net_message_maxsize: i32,
}

/// The DEBUG-only `Sys_Error`s of the send paths (engine-debug feature),
/// for a C frame to raise (ADR-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DgrmError {
    /// "Datagram_SendMessage: zero length message" /
    /// "Datagram_SendUnreliableMessage: zero length message"
    ZeroLength,
    /// "Datagram_SendMessage: message too big: %u" /
    /// "Datagram_SendUnreliableMessage: message too big: %u"
    TooBig(u32),
    /// "SendMessage: called with canSend == false"
    CanSendFalse,
}

#[inline]
fn put_header(pkt: &mut [u8], length_word: u32, sequence: u32) {
    // COMPAT: packetBuffer.length/.sequence = BigLong(x) -- the wire header
    // is big-endian
    pkt[0..4].copy_from_slice(&length_word.to_be_bytes());
    pkt[4..8].copy_from_slice(&sequence.to_be_bytes());
}

#[inline]
fn wire_length_word(pkt: &[u8]) -> u32 {
    u32::from_be_bytes([pkt[0], pkt[1], pkt[2], pkt[3]])
}

#[inline]
fn wire_sequence(pkt: &[u8]) -> u32 {
    u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]])
}

/// Copies `len` bytes starting at `pkt[8 + src_off]` into `dst`, mirroring
/// `memcpy(dst, packetBuffer.data + src_off, len)`. In C a wire-claimed
/// length larger than the scratch reads out of bounds (UB, reachable only
/// from a hostile peer); here the tail beyond the scratch is zero-filled.
/// COMPAT: accepted divergence in that C-UB domain.
fn copy_from_scratch(dst: &mut [u8], pkt: &[u8], len: usize) {
    let avail = pkt.len().saturating_sub(NET_HEADERSIZE).min(len);
    dst[..avail].copy_from_slice(&pkt[NET_HEADERSIZE..NET_HEADERSIZE + avail]);
    dst[avail..len].fill(0);
}

/// INVARIANT (held outside this crate): `sock.max_datagram <=
/// MAX_DATAGRAM`, so `packet_len = NET_HEADERSIZE + data_len` never exceeds
/// `PACKET_BUFFER_SIZE` and the ACK paths' `copy_within` window stays
/// inside `send_message`. The engine maintains it in sv_main.c (which
/// clamps the negotiated MTU) plus `MAX_DATAGRAM == NET_MAXMESSAGE`; a
/// violation would panic in the RX path, so `fuzz_net_dgrm` sweeps the
/// whole legal range rather than trusting the constant.
fn send_fragment<S: NetSys>(
    sys: &mut S,
    sock: &mut QSocket,
    g: &mut DgrmGlobals,
    pkt: &mut [u8],
    sequence: u32,
    resend: bool,
) -> i32 {
    let (data_len, eom);
    if sock.send_message_length <= sock.max_datagram {
        data_len = sock.send_message_length as usize;
        eom = NETFLAG_EOM;
    } else {
        data_len = sock.max_datagram as usize;
        eom = 0;
    }
    let packet_len = NET_HEADERSIZE + data_len;

    put_header(pkt, packet_len as u32 | (NETFLAG_DATA | eom), sequence);
    pkt[NET_HEADERSIZE..packet_len].copy_from_slice(&sock.send_message[..data_len]);

    if sys.write(sock.socket, &pkt[..packet_len], &sock.addr) == -1 {
        return -1;
    }

    sock.last_send_time = g.net_time;
    if resend {
        g.counters.packets_resent = g.counters.packets_resent.wrapping_add(1);
    } else {
        g.counters.packets_sent = g.counters.packets_sent.wrapping_add(1);
    }
    1
}

/// `Datagram_SendMessage`. `data` is the sizebuf contents (`data[..cursize]`).
pub fn send_message<S: NetSys>(
    sys: &mut S,
    sock: &mut QSocket,
    g: &mut DgrmGlobals,
    pkt: &mut [u8],
    data: &[u8],
) -> Result<i32, DgrmError> {
    #[cfg(feature = "engine-debug")]
    {
        if data.is_empty() {
            return Err(DgrmError::ZeroLength);
        }
        if data.len() > NET_MAXMESSAGE {
            return Err(DgrmError::TooBig(data.len() as u32));
        }
        if !sock.can_send {
            return Err(DgrmError::CanSendFalse);
        }
    }
    // release C memcpy's blindly (buffer overflow, UB); the port hard-stops
    // instead. COMPAT: accepted divergence, unreachable -- every engine
    // sizebuf reaching this path is allocated at <= NET_MAXMESSAGE.
    if data.len() > NET_MAXMESSAGE {
        return Err(DgrmError::TooBig(data.len() as u32));
    }

    sock.send_message[..data.len()].copy_from_slice(data);
    sock.send_message_length = data.len() as i32;

    // this can apply only at the start of a reliable, to avoid issues with acks if its resized later.
    sock.max_datagram = sock.pending_max_datagram;

    let sequence = sock.send_sequence;
    sock.send_sequence = sock.send_sequence.wrapping_add(1);
    sock.can_send = false;
    Ok(send_fragment(sys, sock, g, pkt, sequence, false))
}

/// `SendMessageNext`
pub fn send_message_next<S: NetSys>(
    sys: &mut S,
    sock: &mut QSocket,
    g: &mut DgrmGlobals,
    pkt: &mut [u8],
) -> i32 {
    let sequence = sock.send_sequence;
    sock.send_sequence = sock.send_sequence.wrapping_add(1);
    sock.send_next = false;
    send_fragment(sys, sock, g, pkt, sequence, false)
}

/// `ReSendMessage`
pub fn resend_message<S: NetSys>(
    sys: &mut S,
    sock: &mut QSocket,
    g: &mut DgrmGlobals,
    pkt: &mut [u8],
) -> i32 {
    let sequence = sock.send_sequence.wrapping_sub(1);
    send_fragment(sys, sock, g, pkt, sequence, true)
}

/// `Datagram_CanSendMessage` (kicks the pending next fragment first)
pub fn can_send_message<S: NetSys>(
    sys: &mut S,
    sock: &mut QSocket,
    g: &mut DgrmGlobals,
    pkt: &mut [u8],
) -> bool {
    if sock.send_next {
        send_message_next(sys, sock, g, pkt);
    }
    sock.can_send
}

/// `Datagram_CanSendUnreliableMessage`
pub fn can_send_unreliable_message() -> bool {
    true
}

/// `Datagram_SendUnreliableMessage`
pub fn send_unreliable_message<S: NetSys>(
    sys: &mut S,
    sock: &mut QSocket,
    g: &mut DgrmGlobals,
    pkt: &mut [u8],
    data: &[u8],
) -> Result<i32, DgrmError> {
    #[cfg(feature = "engine-debug")]
    {
        if data.is_empty() {
            return Err(DgrmError::ZeroLength);
        }
        if data.len() > quake_types::net::MAX_DATAGRAM {
            return Err(DgrmError::TooBig(data.len() as u32));
        }
    }
    // see send_message: hard-stop where release C would overflow (COMPAT)
    if data.len() > quake_types::net::MAX_DATAGRAM {
        return Err(DgrmError::TooBig(data.len() as u32));
    }

    let packet_len = NET_HEADERSIZE + data.len();

    let sequence = sock.unreliable_send_sequence;
    sock.unreliable_send_sequence = sock.unreliable_send_sequence.wrapping_add(1);
    put_header(pkt, packet_len as u32 | NETFLAG_UNRELIABLE, sequence);
    pkt[NET_HEADERSIZE..packet_len].copy_from_slice(data);

    if sys.write(sock.socket, &pkt[..packet_len], &sock.addr) == -1 {
        return Ok(-1);
    }

    g.counters.packets_sent = g.counters.packets_sent.wrapping_add(1);
    Ok(1)
}

fn sz_clear_net_message(g: &mut DgrmGlobals) {
    *g.net_message_cursize = 0;
}

/// `SZ_Write(&net_message, src, len)` in the RX paths: every call site is
/// pre-checked against maxsize, so the overflow path is unreachable (see
/// module doc / ADR-009 note).
fn sz_write_net_message_from_scratch(g: &mut DgrmGlobals, pkt: &[u8], len: usize) {
    let at = *g.net_message_cursize as usize;
    copy_from_scratch(&mut g.net_message[at..at + len], pkt, len);
    *g.net_message_cursize += len as i32;
}

/// `Datagram_ProcessPacket` -- the server RX path (Datagram_GetAnyMessage
/// reads the packet into `pkt` and hands it here). Returns true when
/// net_message holds a message to parse.
pub fn process_packet<S: NetSys>(
    sys: &mut S,
    sock: &mut QSocket,
    g: &mut DgrmGlobals,
    pkt: &mut [u8],
    length: u32,
) -> bool {
    if (length as usize) < NET_HEADERSIZE {
        g.counters.short_packet_count = g.counters.short_packet_count.wrapping_add(1);
        return false;
    }

    let length_word = wire_length_word(pkt);
    let flags = length_word & !NETFLAG_LENGTH_MASK;
    let length = length_word & NETFLAG_LENGTH_MASK;

    if flags & NETFLAG_CTL != 0 {
        return false; // should only be for OOB packets.
    }

    let sequence = wire_sequence(pkt);
    g.counters.packets_received = g.counters.packets_received.wrapping_add(1);

    if flags & NETFLAG_UNRELIABLE != 0 {
        if sequence < sock.unreliable_receive_sequence {
            sys.dprint("Got a stale datagram\n");
            return false;
        }
        if sequence != sock.unreliable_receive_sequence {
            let count = sequence.wrapping_sub(sock.unreliable_receive_sequence);
            g.counters.dropped_datagrams = g.counters.dropped_datagrams.wrapping_add(count as i32);
            sys.dprint(&format!("Dropped {count} datagram(s)\n"));
        }
        sock.unreliable_receive_sequence = sequence.wrapping_add(1);

        let length = length.wrapping_sub(NET_HEADERSIZE as u32);

        if length > g.net_message_maxsize as u32 {
            // is this even possible? maybe it will be in the future! either way, no sys_errors please.
            sys.print("Over-sized unreliable\n");
            return true;
        }
        sz_clear_net_message(g);
        sz_write_net_message_from_scratch(g, pkt, length as usize);

        *g.unreliable_messages_received = g.unreliable_messages_received.wrapping_add(1);
        return true; // parse the unreliable
    }

    if flags & NETFLAG_ACK != 0 {
        if sequence != sock.send_sequence.wrapping_sub(1) {
            sys.dprint("Stale ACK received\n");
            return false;
        }
        if sequence == sock.ack_sequence {
            sock.ack_sequence = sock.ack_sequence.wrapping_add(1);
            if sock.ack_sequence != sock.send_sequence {
                sys.dprint("ack sequencing error\n");
            }
        } else {
            sys.dprint("Duplicate ACK received\n");
            return false;
        }
        sock.send_message_length -= sock.max_datagram;
        if sock.send_message_length > 0 {
            sock.send_message.copy_within(
                sock.max_datagram as usize..(sock.max_datagram + sock.send_message_length) as usize,
                0,
            );
            sock.send_next = true;
        } else {
            sock.send_message_length = 0;
            sock.can_send = true;
        }
        return false;
    }

    if flags & NETFLAG_DATA != 0 {
        // COMPAT: the ACK reuses the packetBuffer header bytes (clobbering
        // them after the sequence was extracted), and goes to sock->addr --
        // unlike get_message's ACK, which goes to the packet's readaddr
        put_header(pkt, NET_HEADERSIZE as u32 | NETFLAG_ACK, sequence);
        let ack = [
            pkt[0], pkt[1], pkt[2], pkt[3], pkt[4], pkt[5], pkt[6], pkt[7],
        ];
        let addr = sock.addr;
        sys.write(sock.socket, &ack, &addr);

        if sequence != sock.receive_sequence {
            g.counters.received_duplicate_count =
                g.counters.received_duplicate_count.wrapping_add(1);
            return false;
        }
        sock.receive_sequence = sock.receive_sequence.wrapping_add(1);

        let length = length.wrapping_sub(NET_HEADERSIZE as u32) as usize;

        if flags & NETFLAG_EOM != 0 {
            // COMPAT: C computes `receiveMessageLength + length` as a
            // 32-bit unsigned sum, which WRAPS when a hostile wire length
            // below NET_HEADERSIZE makes `length` huge -- C then passes the
            // check and memcpy's a negative length (UB/crash). The port
            // evaluates the sum in usize, so that domain takes this
            // oversize path instead (accepted divergence; the differential
            // avoids the C-crash domain).
            if sock.receive_message_length as usize + length > g.net_message_maxsize as usize {
                sys.print("Over-sized reliable\n");
                return true;
            }
            sz_clear_net_message(g);
            let rml = sock.receive_message_length as usize;
            g.net_message[..rml].copy_from_slice(&sock.receive_message[..rml]);
            *g.net_message_cursize = rml as i32;
            sz_write_net_message_from_scratch(g, pkt, length);
            sock.receive_message_length = 0;

            *g.messages_received = g.messages_received.wrapping_add(1);
            return true; // parse this reliable!
        }

        // COMPAT: C computes `receiveMessageLength + length` as a
        // 32-bit unsigned sum, which WRAPS when a hostile wire length
        // below NET_HEADERSIZE makes `length` huge -- C then passes the
        // check and memcpy's a negative length (UB/crash). The port
        // evaluates the sum in usize, so that domain takes this
        // oversize path instead (accepted divergence; the differential
        // avoids the C-crash domain).
        if sock.receive_message_length as usize + length > sock.receive_message.len() {
            sys.print("Over-sized reliable\n");
            return true;
        }
        let rml = sock.receive_message_length as usize;
        copy_from_scratch(&mut sock.receive_message[rml..rml + length], pkt, length);
        sock.receive_message_length += length as i32;
        return false; // still watiting for the eom
    }
    // unknown flags
    sys.dprint("Unknown packet flags\n");
    false
}

/// get_message's hostile-peer overflow status: the C original reaches
/// `SZ_GetSpace`'s `Host_Error ("SZ_GetSpace: overflow without
/// allowoverflow set")` here; the M7 glue re-raises it from its C frame.
pub const GET_MESSAGE_NET_MESSAGE_OVERFLOW: i32 = -2;

/// `Datagram_GetMessage` -- the client RX path. Returns 0 (nothing),
/// 1 (reliable in net_message), 2 (unreliable in net_message), -1 (error),
/// or [`GET_MESSAGE_NET_MESSAGE_OVERFLOW`].
pub fn get_message<S: NetSys>(
    sys: &mut S,
    sock: &mut QSocket,
    g: &mut DgrmGlobals,
    pkt: &mut [u8],
) -> i32 {
    let mut ret = 0;

    if !sock.can_send && (g.net_time - sock.last_send_time) > 1.0 {
        resend_message(sys, sock, g, pkt);
    }

    loop {
        let (os_length, readaddr) = sys.read(sock.socket, &mut pkt[..PACKET_BUFFER_SIZE]);

        if os_length == 0 {
            break;
        }

        if os_length == -1 {
            sys.print("Read error\n");
            return -1;
        }

        if sys.addr_compare(&readaddr, &sock.addr) != 0 {
            sys.print("Stray/Forged packet received\n");
            let expected = sys.addr_to_string(&sock.addr);
            sys.print(&format!("Expected: {expected}\n"));
            let received = sys.addr_to_string(&readaddr);
            sys.print(&format!("Received: {received}\n"));
            continue;
        }

        if (os_length as u32 as usize) < NET_HEADERSIZE {
            g.counters.short_packet_count = g.counters.short_packet_count.wrapping_add(1);
            continue;
        }

        let length_word = wire_length_word(pkt);
        let flags = length_word & !NETFLAG_LENGTH_MASK;
        let length = length_word & NETFLAG_LENGTH_MASK;

        if flags & NETFLAG_CTL != 0 {
            continue;
        }

        let sequence = wire_sequence(pkt);
        g.counters.packets_received = g.counters.packets_received.wrapping_add(1);

        if flags & NETFLAG_UNRELIABLE != 0 {
            if sequence < sock.unreliable_receive_sequence {
                sys.dprint("Got a stale datagram\n");
                ret = 0;
                break;
            }
            if sequence != sock.unreliable_receive_sequence {
                let count = sequence.wrapping_sub(sock.unreliable_receive_sequence);
                g.counters.dropped_datagrams =
                    g.counters.dropped_datagrams.wrapping_add(count as i32);
                sys.dprint(&format!("Dropped {count} datagram(s)\n"));
            }
            sock.unreliable_receive_sequence = sequence.wrapping_add(1);

            let length = length.wrapping_sub(NET_HEADERSIZE as u32) as usize;

            // COMPAT: unlike process_packet, this path has NO maxsize
            // pre-check in C -- SZ_Write into net_message Host_Errors on a
            // hostile wire length above maxsize (and a header claiming
            // < NET_HEADERSIZE wraps `length` huge, crashing C outright).
            // A longjmp must not cross a Rust frame (ADR-009, M3 shape):
            // return the status and let the C glue frame raise the exact
            // SZ_GetSpace Host_Error. net_message.cursize is already 0 at
            // the C longjmp point (SZ_Clear ran first) -- mirrored here.
            if length > g.net_message_maxsize as usize {
                sz_clear_net_message(g);
                return GET_MESSAGE_NET_MESSAGE_OVERFLOW;
            }
            sz_clear_net_message(g);
            sz_write_net_message_from_scratch(g, pkt, length);

            ret = 2;
            break;
        }

        if flags & NETFLAG_ACK != 0 {
            if sequence != sock.send_sequence.wrapping_sub(1) {
                sys.dprint("Stale ACK received\n");
                continue;
            }
            if sequence == sock.ack_sequence {
                sock.ack_sequence = sock.ack_sequence.wrapping_add(1);
                if sock.ack_sequence != sock.send_sequence {
                    sys.dprint("ack sequencing error\n");
                }
            } else {
                sys.dprint("Duplicate ACK received\n");
                continue;
            }
            sock.send_message_length -= sock.max_datagram;
            if sock.send_message_length > 0 {
                sock.send_message.copy_within(
                    sock.max_datagram as usize
                        ..(sock.max_datagram + sock.send_message_length) as usize,
                    0,
                );
                sock.send_next = true;
            } else {
                sock.send_message_length = 0;
                sock.can_send = true;
            }
            continue;
        }

        if flags & NETFLAG_DATA != 0 {
            put_header(pkt, NET_HEADERSIZE as u32 | NETFLAG_ACK, sequence);
            let ack = [
                pkt[0], pkt[1], pkt[2], pkt[3], pkt[4], pkt[5], pkt[6], pkt[7],
            ];
            sys.write(sock.socket, &ack, &readaddr);

            if sequence != sock.receive_sequence {
                g.counters.received_duplicate_count =
                    g.counters.received_duplicate_count.wrapping_add(1);
                continue;
            }
            sock.receive_sequence = sock.receive_sequence.wrapping_add(1);

            let length = length.wrapping_sub(NET_HEADERSIZE as u32) as usize;

            if flags & NETFLAG_EOM != 0 {
                // COMPAT: C computes `receiveMessageLength + length` as a
                // 32-bit unsigned sum, which WRAPS when a hostile wire length
                // below NET_HEADERSIZE makes `length` huge -- C then passes the
                // check and memcpy's a negative length (UB/crash). The port
                // evaluates the sum in usize, so that domain takes this
                // oversize path instead (accepted divergence; the differential
                // avoids the C-crash domain).
                if sock.receive_message_length as usize + length > g.net_message_maxsize as usize {
                    sys.print("Over-sized reliable\n");
                    return -1;
                }
                sz_clear_net_message(g);
                let rml = sock.receive_message_length as usize;
                g.net_message[..rml].copy_from_slice(&sock.receive_message[..rml]);
                *g.net_message_cursize = rml as i32;
                sz_write_net_message_from_scratch(g, pkt, length);
                sock.receive_message_length = 0;

                ret = 1;
                break;
            }

            // COMPAT: C computes `receiveMessageLength + length` as a
            // 32-bit unsigned sum, which WRAPS when a hostile wire length
            // below NET_HEADERSIZE makes `length` huge -- C then passes the
            // check and memcpy's a negative length (UB/crash). The port
            // evaluates the sum in usize, so that domain takes this
            // oversize path instead (accepted divergence; the differential
            // avoids the C-crash domain).
            if sock.receive_message_length as usize + length > sock.receive_message.len() {
                sys.print("Over-sized reliable\n");
                return -1;
            }
            let rml = sock.receive_message_length as usize;
            copy_from_scratch(&mut sock.receive_message[rml..rml + length], pkt, length);
            sock.receive_message_length += length as i32;
            continue;
        }
    }

    if sock.send_next {
        send_message_next(sys, sock, g, pkt);
    }

    ret
}
