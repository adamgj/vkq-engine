//! `GetWavinfo` (snd_mem.c): the sfx WAV-info parser, ported chunk walk for
//! chunk walk, including the quirky cooledit `cue `/`LIST`+`"mark"` loop
//! parse and the C's bounds clamps (landed there first, harness-verified).
//!
//! The pure function returns the exact `wavinfo_t` the C returns plus an
//! event list of the console messages the C would print (the shim replays
//! them), and a flag for the one `Sys_Error` path ("bad loop length").

use quake_types::sound::{WavInfo, WAV_FORMAT_PCM};

/// A console message `GetWavinfo` would emit, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Msg {
    /// Con_DPrintf2 ("bad \"%s\" chunk length (%d)\n", name, len)
    BadChunkLen { name: &'static str, len: i32 },
    /// Con_Printf ("%s missing RIFF/WAVE chunks\n", name)
    MissingRiffWave,
    /// Con_Printf ("%s is missing fmt chunk\n", name)
    MissingFmt,
    /// Con_Printf ("%s is not Microsoft PCM format\n", name)
    NotPcm,
    /// Con_Printf ("%s is missing data chunk\n", name)
    MissingData,
    /// Con_Warning ("%s has loop start >= end\n", name)
    LoopStartGeEnd,
}

#[derive(Debug)]
pub struct WavParse {
    /// exactly the `wavinfo_t` the C function returns (partial on reject)
    pub info: WavInfo,
    pub messages: Vec<Msg>,
    /// the C calls `Sys_Error ("%s has a bad loop length", name)`
    pub bad_loop_length: bool,
}

struct Iff<'a> {
    wav: &'a [u8],
    iff_data: usize,
    last_chunk: usize,
    /// C's data_p: index of the current chunk header, None when not found
    data_p: Option<usize>,
    chunk_len: i32,
}

fn le16(wav: &[u8], p: usize) -> i16 {
    i16::from_le_bytes([wav[p], wav[p + 1]])
}

fn le32(wav: &[u8], p: usize) -> i32 {
    i32::from_le_bytes([wav[p], wav[p + 1], wav[p + 2], wav[p + 3]])
}

impl<'a> Iff<'a> {
    fn find_next_chunk(&mut self, name: &'static str, msgs: &mut Vec<Msg>) {
        loop {
            // C: if (last_chunk + 8 >= iff_end)
            if self.last_chunk + 8 >= self.wav.len() {
                self.data_p = None;
                return;
            }
            let p = self.last_chunk + 4;
            let len = le32(self.wav, p);
            // C: if (iff_chunk_len < 0 || iff_chunk_len > iff_end - data_p)
            if len < 0 || len as i64 > (self.wav.len() - (p + 4)) as i64 {
                self.data_p = None;
                msgs.push(Msg::BadChunkLen { name, len });
                return;
            }
            self.last_chunk = p + 4 + ((len as usize + 1) & !1);
            let hdr = p - 4;
            if &self.wav[hdr..hdr + 4] == name.as_bytes() {
                self.data_p = Some(hdr);
                self.chunk_len = len;
                return;
            }
        }
    }

    fn find_chunk(&mut self, name: &'static str, msgs: &mut Vec<Msg>) {
        self.last_chunk = self.iff_data;
        self.find_next_chunk(name, msgs);
    }
}

pub fn get_wavinfo(wav: &[u8]) -> WavParse {
    let mut out = WavParse {
        info: WavInfo::default(),
        messages: Vec::new(),
        bad_loop_length: false,
    };
    let info = &mut out.info;
    let msgs = &mut out.messages;

    // C: if (!wav) return info -- a null pointer, not an empty file; the
    // shim handles null before calling here

    let mut iff = Iff {
        wav,
        iff_data: 0,
        last_chunk: 0,
        data_p: None,
        chunk_len: 0,
    };

    // find "RIFF" chunk (bounds clamp: need 4 bytes of chunk data for "WAVE")
    iff.find_chunk("RIFF", msgs);
    let riff_ok = match iff.data_p {
        Some(p) => iff.chunk_len >= 4 && &wav[p + 8..p + 12] == b"WAVE",
        None => false,
    };
    if !riff_ok {
        msgs.push(Msg::MissingRiffWave);
        return out;
    }

    // get "fmt " chunk
    iff.iff_data = iff.data_p.unwrap() + 12;
    iff.find_chunk("fmt ", msgs);
    // (bounds clamp: the fields read below span 16 bytes of chunk data)
    let fmt = match iff.data_p {
        Some(p) if iff.chunk_len >= 16 => p,
        _ => {
            msgs.push(Msg::MissingFmt);
            return out;
        }
    };
    let mut p = fmt + 8;
    let format = le16(wav, p) as i32;
    p += 2;
    if format != WAV_FORMAT_PCM {
        msgs.push(Msg::NotPcm);
        return out;
    }
    info.channels = le16(wav, p) as i32;
    p += 2;
    info.rate = le32(wav, p);
    p += 4;
    p += 4 + 2;
    let bits = le16(wav, p) as i32;
    if bits != 8 && bits != 16 {
        return out;
    }
    info.width = bits / 8;

    // get cue chunk (bounds clamp: the loopstart read is at data offset 24)
    iff.find_chunk("cue ", msgs);
    if iff.data_p.is_some() && iff.chunk_len >= 28 {
        let p = iff.data_p.unwrap() + 32;
        info.loopstart = le32(wav, p);

        // if the next chunk is a LIST chunk, look for a cue length marker
        iff.find_next_chunk("LIST", msgs);
        if let Some(p) = iff.data_p {
            if iff.chunk_len >= 32 && &wav[p + 28..p + 32] == b"mark" {
                // this is not a proper parse, but it works with cooledit...
                let samples_in_loop = le32(wav, p + 24);
                info.samples = info.loopstart.wrapping_add(samples_in_loop);
            }
        }
    } else {
        info.loopstart = -1;
    }

    // find data chunk
    iff.find_chunk("data", msgs);
    let Some(p) = iff.data_p else {
        msgs.push(Msg::MissingData);
        return out;
    };

    // C: data_p += 4; samples = GetLittleLong () / info.width
    // (re-reads the chunk length field from the header)
    let samples = le32(wav, p + 4).wrapping_div(info.width);

    if info.samples != 0 {
        if samples < info.samples {
            out.bad_loop_length = true;
            return out;
        }
    } else {
        info.samples = samples;
    }

    if info.loopstart >= info.samples {
        msgs.push(Msg::LoopStartGeEnd);
        info.loopstart = -1;
        info.samples = samples;
    }

    info.dataofs = (p + 8) as i32;

    out
}
