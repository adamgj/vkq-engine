//! Loopback driver frame format (net_loop.c, Phase 5 M5): the in-memory
//! message queue packed into a peer's `receiveMessage` buffer as
//! `[type u8][len u16 LE][pad u8]` (+ 4-byte LE sequence for unreliables),
//! 4-byte aligned. The qsocket plumbing (statics, driverdata peering,
//! canSend cross-set) lives in the capi driver; this module owns the byte
//! format so it is testable and fuzzable without FFI.

use quake_types::net::{NET_LOOPBACKBUFFERS, NET_LOOPBACKHEADERSIZE, NET_MAXMESSAGE};

pub const TYPE_RELIABLE: u8 = 1;
pub const TYPE_UNRELIABLE: u8 = 2;

/// `IntAlign`
#[inline]
pub fn int_align(value: i32) -> i32 {
    (value + 3) & !3
}

/// `Loop_SendMessage`'s buffer append. Returns false for the
/// `Sys_Error ("Loop_SendMessage: overflow")` path.
#[must_use]
pub fn push_reliable(buf: &mut [u8], buffer_length: &mut i32, payload: &[u8]) -> bool {
    let cursize = payload.len() as i32;
    if *buffer_length + cursize + NET_LOOPBACKHEADERSIZE as i32
        > (NET_MAXMESSAGE * NET_LOOPBACKBUFFERS + NET_LOOPBACKHEADERSIZE) as i32
    {
        return false;
    }
    let at = *buffer_length as usize;
    buf[at] = TYPE_RELIABLE;
    buf[at + 1] = (cursize & 0xff) as u8;
    buf[at + 2] = (cursize >> 8) as u8;
    // buf[at + 3] is the alignment byte (left as-is, like C)
    buf[at + 4..at + 4 + payload.len()].copy_from_slice(payload);
    *buffer_length = int_align(*buffer_length + cursize + 4);
    true
}

/// `Loop_SendUnreliableMessage`'s buffer append. Returns false for the
/// "would eat the reserved reliable buffer" refusal (C returns 0).
pub fn push_unreliable(
    buf: &mut [u8],
    buffer_length: &mut i32,
    payload: &[u8],
    sequence: u32,
) -> bool {
    let cursize = payload.len() as i32;
    // always leave one buffer for reliable messages
    if *buffer_length + cursize + NET_LOOPBACKHEADERSIZE as i32
        > (NET_MAXMESSAGE * (NET_LOOPBACKBUFFERS - 1)) as i32
    {
        return false;
    }
    let at = *buffer_length as usize;
    buf[at] = TYPE_UNRELIABLE;
    buf[at + 1] = (cursize & 0xff) as u8;
    buf[at + 2] = (cursize >> 8) as u8;
    // buf[at + 3] is the alignment byte
    buf[at + 4..at + 8].copy_from_slice(&sequence.to_le_bytes());
    buf[at + 8..at + 8 + payload.len()].copy_from_slice(payload);
    *buffer_length = int_align(*buffer_length + cursize + 8);
    true
}

/// One popped frame: `msg_type` is the C return value (1 reliable, 2
/// unreliable), `payload` the range within the buffer AS IT WAS before the
/// remainder was shifted down, `new_unreliable_receive_sequence` the
/// post-increment sequence for type 2.
pub struct Popped {
    pub msg_type: i32,
    pub payload_start: usize,
    pub payload_len: usize,
    pub new_unreliable_receive_sequence: Option<u32>,
}

/// `Loop_GetMessage`'s dequeue, minus the `net_message` copy and peer
/// canSend cross-set (capi side). The caller must copy
/// `buf[payload_start..payload_start+payload_len]` out BEFORE calling
/// `pop_finish`, which shifts the remainder down.
pub fn pop_peek(buf: &[u8], buffer_length: i32) -> Option<Popped> {
    if buffer_length == 0 {
        return None;
    }
    let msg_type = buf[0] as i32;
    let length = buf[1] as i32 + ((buf[2] as i32) << 8);
    // alignment byte skipped here
    if msg_type == TYPE_UNRELIABLE as i32 {
        let seq = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        Some(Popped {
            msg_type,
            payload_start: 8,
            payload_len: length as usize,
            new_unreliable_receive_sequence: Some(seq.wrapping_add(1)),
        })
    } else {
        Some(Popped {
            msg_type,
            payload_start: 4,
            payload_len: length as usize,
            new_unreliable_receive_sequence: None,
        })
    }
}

/// The dequeue's buffer-shift half: consumes the aligned frame and memmoves
/// the remainder to the front, returning the new buffer length.
pub fn pop_finish(buf: &mut [u8], buffer_length: i32, p: &Popped) -> i32 {
    let consumed = int_align(p.payload_len as i32 + p.payload_start as i32);
    let remaining = buffer_length - consumed;
    if remaining > 0 {
        buf.copy_within(consumed as usize..(consumed + remaining) as usize, 0);
    }
    remaining
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_roundtrip_with_alignment() {
        let mut buf = vec![0u8; NET_MAXMESSAGE * NET_LOOPBACKBUFFERS + NET_LOOPBACKHEADERSIZE];
        let mut len = 0i32;
        assert!(push_reliable(&mut buf, &mut len, b"abc"));
        assert_eq!(len, int_align(3 + 4));
        assert!(push_unreliable(&mut buf, &mut len, b"defg", 41));
        assert!(push_reliable(&mut buf, &mut len, b""));

        let p = pop_peek(&buf, len).unwrap();
        assert_eq!(p.msg_type, 1);
        assert_eq!(
            &buf[p.payload_start..p.payload_start + p.payload_len],
            b"abc"
        );
        len = pop_finish(&mut buf, len, &p);

        let p = pop_peek(&buf, len).unwrap();
        assert_eq!(p.msg_type, 2);
        assert_eq!(p.new_unreliable_receive_sequence, Some(42));
        assert_eq!(
            &buf[p.payload_start..p.payload_start + p.payload_len],
            b"defg"
        );
        len = pop_finish(&mut buf, len, &p);

        let p = pop_peek(&buf, len).unwrap();
        assert_eq!(p.msg_type, 1);
        assert_eq!(p.payload_len, 0);
        len = pop_finish(&mut buf, len, &p);
        assert_eq!(len, 0);
        assert!(pop_peek(&buf, len).is_none());
    }

    #[test]
    fn bounds_match_c() {
        let cap = NET_MAXMESSAGE * NET_LOOPBACKBUFFERS + NET_LOOPBACKHEADERSIZE;
        let mut buf = vec![0u8; cap];
        // reliable: hard Sys_Error bound at the full buffer
        let mut len = (cap - NET_LOOPBACKHEADERSIZE) as i32;
        assert!(push_reliable(&mut buf, &mut len, b""));
        let mut len = (cap - NET_LOOPBACKHEADERSIZE) as i32 + 1;
        assert!(!push_reliable(&mut buf, &mut len, b""));
        // unreliable: refuses once inside the reserved last buffer
        let mut len = (NET_MAXMESSAGE * (NET_LOOPBACKBUFFERS - 1)) as i32 - 4;
        assert!(push_unreliable(&mut buf, &mut len, b"", 0));
        let mut len = (NET_MAXMESSAGE * (NET_LOOPBACKBUFFERS - 1)) as i32 - 3;
        assert!(!push_unreliable(&mut buf, &mut len, b"", 0));
    }
}
