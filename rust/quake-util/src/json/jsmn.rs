//! Hand-port of `Quake/jsmn.h` exactly as json.c configures it:
//! `JSMN_PARENT_LINKS` on, **`JSMN_STRICT` off** — the tolerant,
//! non-validating behavior is the spec (ADR-012: MD5-metadata/mod-content
//! acceptance must not change; no serde_json). The strict-mode-only branches
//! are compiled out in C and correspondingly absent here.

pub const JSMN_UNDEFINED: i32 = 0;
pub const JSMN_OBJECT: i32 = 1 << 0;
pub const JSMN_ARRAY: i32 = 1 << 1;
pub const JSMN_STRING: i32 = 1 << 2;
pub const JSMN_PRIMITIVE: i32 = 1 << 3;

pub const JSMN_ERROR_NOMEM: i32 = -1;
pub const JSMN_ERROR_INVAL: i32 = -2;
pub const JSMN_ERROR_PART: i32 = -3;

#[derive(Debug, Clone, Copy)]
pub struct JsmnTok {
    pub type_: i32,
    pub start: i32,
    pub end: i32,
    pub size: i32,
    pub parent: i32,
}

pub struct JsmnParser {
    pos: usize,
    toknext: usize,
    toksuper: i32,
}

impl Default for JsmnParser {
    fn default() -> Self {
        Self::new()
    }
}

impl JsmnParser {
    pub fn new() -> Self {
        JsmnParser {
            pos: 0,
            toknext: 0,
            toksuper: -1,
        }
    }

    fn alloc_token<'a>(&mut self, tokens: &'a mut [JsmnTok]) -> Option<&'a mut JsmnTok> {
        if self.toknext >= tokens.len() {
            return None;
        }
        let tok = &mut tokens[self.toknext];
        self.toknext += 1;
        tok.start = -1;
        tok.end = -1;
        tok.size = 0;
        tok.parent = -1;
        Some(tok)
    }

    fn parse_primitive(&mut self, js: &[u8], len: usize, tokens: Option<&mut [JsmnTok]>) -> i32 {
        let start = self.pos;

        while self.pos < len && js[self.pos] != 0 {
            match js[self.pos] {
                // non-strict mode: ':' also terminates a primitive
                b':' | b'\t' | b'\r' | b'\n' | b' ' | b',' | b']' | b'}' => break,
                _ => {}
            }
            // C: `js[pos] < 32 || js[pos] >= 127` on (signed) char — both
            // signed and unsigned reads reject bytes >= 0x7f
            if js[self.pos] < 32 || js[self.pos] >= 127 {
                self.pos = start;
                return JSMN_ERROR_INVAL;
            }
            self.pos += 1;
        }
        // non-strict mode: EOF also ends a primitive (falls through to found)

        let Some(tokens) = tokens else {
            self.pos -= 1;
            return 0;
        };
        match self.alloc_token(tokens) {
            None => {
                self.pos = start;
                JSMN_ERROR_NOMEM
            }
            Some(token) => {
                token.type_ = JSMN_PRIMITIVE;
                token.start = start as i32;
                token.end = self.pos as i32;
                token.size = 0;
                token.parent = self.toksuper;
                self.pos -= 1;
                0
            }
        }
    }

    fn parse_string(&mut self, js: &[u8], len: usize, tokens: Option<&mut [JsmnTok]>) -> i32 {
        let start = self.pos;

        // skip starting quote
        self.pos += 1;

        while self.pos < len && js[self.pos] != 0 {
            let c = js[self.pos];

            // quote: end of string
            if c == b'"' {
                let Some(tokens) = tokens else {
                    return 0;
                };
                return match self.alloc_token(tokens) {
                    None => {
                        self.pos = start;
                        JSMN_ERROR_NOMEM
                    }
                    Some(token) => {
                        token.type_ = JSMN_STRING;
                        token.start = start as i32 + 1;
                        token.end = self.pos as i32;
                        token.size = 0;
                        token.parent = self.toksuper;
                        0
                    }
                };
            }

            // backslash: quoted symbol expected
            if c == b'\\' && self.pos + 1 < len {
                self.pos += 1;
                match js[self.pos] {
                    b'"' | b'/' | b'\\' | b'b' | b'f' | b'r' | b'n' | b't' => {}
                    b'u' => {
                        self.pos += 1;
                        let mut i = 0;
                        while i < 4 && self.pos < len && js[self.pos] != 0 {
                            if !js[self.pos].is_ascii_hexdigit() {
                                self.pos = start;
                                return JSMN_ERROR_INVAL;
                            }
                            self.pos += 1;
                            i += 1;
                        }
                        self.pos -= 1;
                    }
                    _ => {
                        self.pos = start;
                        return JSMN_ERROR_INVAL;
                    }
                }
            }
            self.pos += 1;
        }
        self.pos = start;
        JSMN_ERROR_PART
    }

    /// C `jsmn_parse`. `tokens = None` is the counting pass.
    pub fn parse(&mut self, js: &[u8], len: usize, mut tokens: Option<&mut [JsmnTok]>) -> i32 {
        let mut count = self.toknext as i32;

        while self.pos < len && js[self.pos] != 0 {
            let c = js[self.pos];
            match c {
                b'{' | b'[' => {
                    count += 1;
                    if let Some(tokens) = tokens.as_deref_mut() {
                        let toksuper = self.toksuper;
                        let toknext = self.toknext;
                        let Some(token) = self.alloc_token(tokens) else {
                            return JSMN_ERROR_NOMEM;
                        };
                        token.type_ = if c == b'{' { JSMN_OBJECT } else { JSMN_ARRAY };
                        token.start = self.pos as i32;
                        token.parent = if toksuper != -1 { toksuper } else { -1 };
                        if toksuper != -1 {
                            tokens[toksuper as usize].size += 1;
                        }
                        self.toksuper = toknext as i32;
                    }
                }
                b'}' | b']' => {
                    if let Some(tokens) = tokens.as_deref_mut() {
                        let type_ = if c == b'}' { JSMN_OBJECT } else { JSMN_ARRAY };
                        if self.toknext < 1 {
                            return JSMN_ERROR_INVAL;
                        }
                        let mut ti = self.toknext - 1;
                        loop {
                            let token = tokens[ti];
                            if token.start != -1 && token.end == -1 {
                                if token.type_ != type_ {
                                    return JSMN_ERROR_INVAL;
                                }
                                tokens[ti].end = self.pos as i32 + 1;
                                self.toksuper = token.parent;
                                break;
                            }
                            if token.parent == -1 {
                                if token.type_ != type_ || self.toksuper == -1 {
                                    return JSMN_ERROR_INVAL;
                                }
                                break;
                            }
                            ti = token.parent as usize;
                        }
                    }
                }
                b'"' => {
                    let r = self.parse_string(js, len, tokens.as_deref_mut());
                    if r < 0 {
                        return r;
                    }
                    count += 1;
                    if self.toksuper != -1 {
                        if let Some(tokens) = tokens.as_deref_mut() {
                            tokens[self.toksuper as usize].size += 1;
                        }
                    }
                }
                b'\t' | b'\r' | b'\n' | b' ' => {}
                b':' => {
                    self.toksuper = self.toknext as i32 - 1;
                }
                b',' => {
                    if let Some(tokens) = tokens.as_deref_mut() {
                        if self.toksuper != -1
                            && tokens[self.toksuper as usize].type_ != JSMN_ARRAY
                            && tokens[self.toksuper as usize].type_ != JSMN_OBJECT
                        {
                            self.toksuper = tokens[self.toksuper as usize].parent;
                        }
                    }
                }
                // non-strict mode: every other unquoted value is a primitive
                _ => {
                    let r = self.parse_primitive(js, len, tokens.as_deref_mut());
                    if r < 0 {
                        return r;
                    }
                    count += 1;
                    if self.toksuper != -1 {
                        if let Some(tokens) = tokens.as_deref_mut() {
                            tokens[self.toksuper as usize].size += 1;
                        }
                    }
                }
            }
            self.pos += 1;
        }

        if let Some(tokens) = tokens {
            for i in (0..self.toknext).rev() {
                // unmatched opened object or array
                if tokens[i].start != -1 && tokens[i].end == -1 {
                    return JSMN_ERROR_PART;
                }
            }
        }

        count
    }
}
