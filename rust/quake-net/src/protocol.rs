//! Protocol identifiers and flag sets (Quake/protocol.h) the wire layer
//! dispatches on. Values are ABI: they select on-wire encodings.

pub const PROTOCOL_NETQUAKE: u32 = 15;
pub const PROTOCOL_FITZQUAKE: u32 = 666;
pub const PROTOCOL_RMQ: u32 = 999;

// PROTOCOL_RMQ protocol flags
pub const PRFL_SHORTANGLE: u32 = 1 << 1;
pub const PRFL_FLOATANGLE: u32 = 1 << 2;
pub const PRFL_24BITCOORD: u32 = 1 << 3;
pub const PRFL_FLOATCOORD: u32 = 1 << 4;
pub const PRFL_EDICTSCALE: u32 = 1 << 5;
pub const PRFL_ALPHASANITY: u32 = 1 << 6;
pub const PRFL_INT32COORD: u32 = 1 << 7;
pub const PRFL_MOREFLAGS: u32 = 1 << 31;

// PROTOCOL_FTE_PEXT2 flags (the wire layer only dispatches on
// REPLACEMENTDELTAS; the rest ride along for callers)
pub const PEXT2_PRYDONCURSOR: u32 = 0x00000001;
pub const PEXT2_VOICECHAT: u32 = 0x00000002;
pub const PEXT2_REPLACEMENTDELTAS: u32 = 0x00000008;
pub const PEXT2_PREDINFO: u32 = 0x00000020;
