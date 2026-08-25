#!/usr/bin/env python3
"""Minimal hand-crafted JPEG fixtures for the M8 image_crate_differential.

Custom tiny Huffman tables (canonical):
  DC table 0: 1 code of length 1: symbol 0x00 (category 0) -> "0"
  AC table 0: len1: 0x00 (EOB) -> "0"; len2: 0x01 (run 0, size 1) -> "10"
Quant table 0: all ones. All components share table ids 0.
Blocks are DC-diff-0 with an optional single AC(0,1)=+1 coefficient.
"""
import struct

def seg(marker, payload):
    return bytes([0xFF, marker]) + struct.pack('>H', len(payload) + 2) + payload

def dqt():
    return seg(0xDB, bytes([0x00]) + bytes([1] * 64))

def dht():
    dc = bytes([0x00]) + bytes([1] + [0] * 15) + bytes([0x00])
    ac = bytes([0x10]) + bytes([1, 1] + [0] * 14) + bytes([0x00, 0x01])
    return seg(0xC4, dc) + seg(0xC4, ac)

def sof(marker, w, h, comps):
    # comps: list of (id, h_samp, v_samp, qtab)
    p = bytes([8]) + struct.pack('>HH', h, w) + bytes([len(comps)])
    for cid, hs, vs, q in comps:
        p += bytes([cid, (hs << 4) | vs, q])
    return seg(marker, p)

def sos(comp_ids, ss=0, se=63, ah=0, al=0):
    p = bytes([len(comp_ids)])
    for cid in comp_ids:
        p += bytes([cid, 0x00])  # DC table 0, AC table 0
    p += bytes([ss, se, (ah << 4) | al])
    return seg(0xDA, p)

class Bits:
    def __init__(self):
        self.bits = []
    def put(self, s):
        self.bits.extend(int(c) for c in s)
    def bytes_(self):
        bits = self.bits[:]
        while len(bits) % 8:
            bits.append(1)  # pad with 1s
        out = bytearray()
        for i in range(0, len(bits), 8):
            b = 0
            for bit in bits[i:i + 8]:
                b = (b << 1) | bit
            out.append(b)
            if b == 0xFF:
                out.append(0x00)  # byte stuffing
        return bytes(out)

BLOCK_AC = "0" + "10" + "1" + "0"   # DC cat0, AC(0,1)=+1, EOB
BLOCK_FLAT = "0" + "0"               # DC cat0, EOB

def entropy(blocks):
    b = Bits()
    for blk in blocks:
        b.put(blk)
    return b.bytes_()

SOI = b'\xff\xd8'
EOI = b'\xff\xd9'

fx = {}
# 1. baseline grayscale 8x8, one AC coefficient (a real cosine ripple)
fx['gray8'] = SOI + dqt() + sof(0xC0, 8, 8, [(1, 1, 1, 0)]) + dht() + sos([1]) + entropy([BLOCK_AC]) + EOI
# 2. baseline color 8x8 4:4:4
fx['rgb444'] = SOI + dqt() + sof(0xC0, 8, 8, [(1, 1, 1, 0), (2, 1, 1, 0), (3, 1, 1, 0)]) + dht() + sos([1, 2, 3]) + entropy([BLOCK_AC, BLOCK_FLAT, BLOCK_AC]) + EOI
# 3. baseline color 16x16 4:2:0 (one MCU: 4 Y + Cb + Cr)
fx['rgb420'] = SOI + dqt() + sof(0xC0, 16, 16, [(1, 2, 2, 0), (2, 1, 1, 0), (3, 1, 1, 0)]) + dht() + sos([1, 2, 3]) + entropy([BLOCK_AC, BLOCK_FLAT, BLOCK_AC, BLOCK_FLAT, BLOCK_AC, BLOCK_FLAT]) + EOI
# 4. baseline color 16x8 4:2:2
fx['rgb422'] = SOI + dqt() + sof(0xC0, 16, 8, [(1, 2, 1, 0), (2, 1, 1, 0), (3, 1, 1, 0)]) + dht() + sos([1, 2, 3]) + entropy([BLOCK_AC, BLOCK_FLAT, BLOCK_AC, BLOCK_FLAT]) + EOI
# 5. odd dims (9x5): MCU padding on both axes
fx['odd'] = SOI + dqt() + sof(0xC0, 9, 5, [(1, 1, 1, 0)]) + dht() + sos([1]) + entropy([BLOCK_AC, BLOCK_FLAT]) + EOI
# 6. restart markers: 24x8 gray, DRI=1, RST between the 3 MCUs
dri = seg(0xDD, struct.pack('>H', 1))
scan = entropy([BLOCK_AC]) + b'\xff\xd0' + entropy([BLOCK_FLAT]) + b'\xff\xd1' + entropy([BLOCK_AC])
fx['restart'] = SOI + dqt() + dri + sof(0xC0, 24, 8, [(1, 1, 1, 0)]) + dht() + sos([1]) + scan + EOI
# 7. APP1/EXIF-ish + comment segments skipped
app1 = seg(0xE1, b'Exif\x00\x00junkjunk')
com = seg(0xFE, b'a comment')
fx['appn'] = SOI + app1 + com + dqt() + sof(0xC0, 8, 8, [(1, 1, 1, 0)]) + dht() + sos([1]) + entropy([BLOCK_AC]) + EOI
# 8. progressive: SOF2, DC-only first scan, then EOI (no AC scans)
fx['progressive_dc'] = SOI + dqt() + sof(0xC2, 8, 8, [(1, 1, 1, 0)]) + dht() + sos([1], ss=0, se=0) + entropy([BLOCK_FLAT[:1]]) + EOI
# 9. truncated: rgb420 cut mid-entropy-data
full = fx['rgb420']
fx['truncated'] = full[:len(full) - 4]
# 10. garbage after EOI
fx['trailing'] = fx['gray8'] + b'garbage after the end of image'
# 11. SOI then garbage (sniffs as JPEG, decode fails)
fx['soi_garbage'] = SOI + b'\x00\x01\x02not a jpeg at all'

for name, data in fx.items():
    arr = ', '.join(str(b) for b in data)
    print(f'const JPEG_{name.upper()}: &[u8] = &[{arr}];')
