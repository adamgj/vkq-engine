#!/usr/bin/env python3
"""4-way interop matrix (Phase 5 M8, ADR-019 gate 4 / phase-5 exit criterion).

Runs C/Rust client x C/Rust server localhost sessions across the protocol
cells this server can negotiate:

  Base-15   protocol 15, no FTE extensions      FTE+15   15  + PEXT2
  Base-666  protocol 666, no FTE extensions     FTE+666  666 + PEXT2
  Base-999  999 (PRFL_INT32COORD|SHORTANGLE)    FTE+999  999 + PEXT2
                                                         (PRFL_FLOATCOORD|SHORTANGLE)

Per cell x build-combo the gate checks: the client negotiated the expected
protocol string, the session produced healthy traffic, and -- across the
four build combos of a cell -- the reliable record counts match exactly and
the unreliable counts sit within the live-session noise floor. The
msgbadread counters are compared C-vs-Rust per the M1 amendment (benign
events occur by design on the dgrm connect path).

Live sessions carry run-to-run timing noise, so byte-exactness is NOT
asserted here -- that is netreplay_diff.py's job. This matrix proves the
cross-implementation handshake+session health that a replay cannot.

Usage:
  interop_matrix.py --vkquake-c <exe> --vkquake-rs <exe>
                    [--game-data <dir>] [--frames 600] [--cells ...]
                    [--ipv6] [--base-port 26100]

--soak: netplay-soak gate (Phase 7 M1 T1.3; plan doc
"Soak definition" in docs/ai/plans/rust-conversion-phase-7.md).

Runs the same dedicated-server + scripted-headless-client machinery above,
extended to a frame-count-bounded soak with a periodic *state-hash* desync
check instead of the matrix's packet-count health check:

  interop_matrix.py --soak --vkquake-c <exe> --vkquake-rs <exe>
                    [--game-data <dir>] [--frames 20000|100000]
                    [--protocols Base-666 FTE+999] [--combos ...]
                    [--hash-interval 64] [--results-dir DIR]

Cells: 4 build combos (C/C, Csv/Rcl, Rsv/Ccl, R/R) x 2 protocols
(Base-666, FTE+999) = 8 cells. Frame budget is server-frame-count based
(--frames; smoke default 20000, full soak 100000 per the plan's soak
budget decision). A dedicated server IS fixed-dt under -demohash (see FIXED
GAPS below -- Quake/main_sdl.c:133-135, added with this gate), so the hash
chain is deterministic in simulated time; only the *outer* loop still paces
to real time, so wall-clock duration tracks a sys_ticrate-throttled pace
rather than simulated time.

Why this gate is not hash-exact
------------------------------
It was, and it could not work. The server's own simulation IS deterministic
(a dedicated server run under -demohash is fixed-dt), but only *given its
input*. Which server frame each live UDP datagram from a separately-launched
client lands on is scheduler- and socket-buffer-dependent, and a single
one-frame shift in when a clc_move is applied forks the two simulations
permanently. Measured directly on identical binaries, same map, zero
scripted player input and no injected fault: two clean C/C sessions fork at
~frame 128 and never reconverge. That also means no fault injected after the
natural fork point could ever have been *attributed* to the injection -- so
the exact-hash model could neither pass cleanly nor prove itself, which is
why it was replaced (Phase 7 M1 T1.8 finding; see the plan's amendment log).

Per-frame simulation equivalence is already gated, properly and exactly, by
run_corpus.py's single-process fixed-dt hash goldens. This gate covers what
only a live two-process session can: that a long netplay session stays up
and keeps a sane packet profile.

Pass criterion (per cell, all must hold):
  * no Host_Error/Sys_Error on either side (detected via the
    "QUAKE ERROR:"/"Host_Error:" markers the C code already prints), no
    crash, no client timeout, no unexpected exit code;
  * the negotiated protocol is the expected one;
  * **liveness**: the cell's server reached every --hash-interval (default
    64) checkpoint frame its protocol's C/C reference reached. A missing
    checkpoint means the cell's server died, crashed or exited early. Note
    it does NOT mean the client was dropped: the hash stream comes from the
    dedicated server, which runs to -exitafter whether or not a client is
    still attached. A dropped client is caught by the traffic profile below
    plus the client-side Host_Error scan. (--inject-desync-at does not
    distinguish the two -- net_messagetimeout 0 trips the traffic condition,
    not this one.);
  * **traffic profile** within the same tolerance the non-soak matrix gate
    uses: reliable counts exact, unreliable counts within +-max(6, 10%), and
    msgbadread judged by its offset from the received-message count.

The per-frame state-hash stream is still captured (via the engine's own
-demohash flag on the server; harness.c already emits "F <frame> <hash>"
per simulated frame, this script only reads that file) and the first
diverging checkpoint is *reported* as a diagnostic -- a large shift in the
fork point between builds is a useful smell -- but it is never pass/fail.

A separate C/C reference is still generated once per protocol: the
negotiated wire protocol measurably changes both streams, so one protocol's
reference is not valid for another.

Client input stays exactly what the matrix above already does (a
frame-numbered `harness.cmds` script executed by Harness_Frame) --
--soak does not add scripted movement, only (optionally) a `record`
command so a failing cell leaves a client-side demo behind.

On any cell failure this dumps to --results-dir: the last-N checkpoint
hash window from both streams, the exact failing server-frame index, the
server log, the client's stdout, both harness.cmds scripts (for exact
repro), and -- when the client survived long enough to have issued its
scripted `record` -- the client-side demo of the failing session (there is
no server-side demo mechanism in this engine; dedicated servers cannot
record, only clients can, per cl_demo.c) as the offline-replay artifact.

Fault-injection proof hook (--inject-desync-at FRAME, hidden from --help):
appends "<FRAME> net_messagetimeout 0" to the *server*'s harness.cmds for
every comparison cell (never to the reference-stream generation, which must
stay clean) -- a config skew, per the plan's suggested mechanism, never a
raw-byte hack. The server's own timeout check (net_dgrm.c:204) then fires on
its next frame and drops the connected client, which the gate observes as a
truncated hash stream and/or a collapsed packet profile. A soak run with
this flag set that reports zero failures means the detector did not fire,
and is the redundant half of the M1 gate ("a detector that has never fired
is not a gate"). Override via --inject-cvar/--inject-value; note that a
purely simulation-affecting skew (the original sv_gravity 400) is NOT a
valid injection for this gate any more -- see "Why this gate is not
hash-exact" above.

FIXED GAPS (Phase 7 M1 T1.8; both were found by running this gate for real
and are recorded in the plan's amendment log):
  * Quake/main_sdl.c -- the dedicated loop now applies
    harness_fixed_dt/Harness_FrameTime() to the value fed into Host_Frame
    while still pacing its outer wait to real wall-clock time via
    sys_ticrate; and the *non-dedicated* loop now paces itself the same way
    when harness_active && !harness_fixed_dt (this gate's live client),
    which previously raced through -exitafter frames unthrottled and
    flooded the server's socket.
  * Quake/net_wins.c WINS_Read -- WSAECONNRESET on a connectionless UDP
    socket (Windows' ICMP-port-unreachable echo) was logged but still
    returned -1, which net_dgrm_rel.c treats as a fatal read error and
    drops the session outright. It now returns 0 like the other transient
    errnos. This was killing soak sessions mid-run with
    "Host_Error: CL_ReadFromServer: lost server connection".
"""

import argparse
import errno
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time

CELLS = ["Base-15", "FTE+15", "Base-666", "FTE+666", "Base-999", "FTE+999"]
EXPECT_PROTO = {
    "Base-15": "15",
    "FTE+15": "fte15",
    "Base-666": "666",
    "FTE+666": "fte666",
    "Base-999": "999",
    "FTE+999": "fte999",
}

# --soak: 4 build combos x 2 protocols (plan's "Soak definition"). The
# protocol pair is deliberately a subset of CELLS -- state-hash comparisons
# across protocols are validated once via the shared reference stream (see
# module docstring), so soaking every CELLS entry would only re-prove wire
# encoding the matrix above already covers, at 3x the wall-clock cost.
SOAK_PROTOCOLS = ["Base-666", "FTE+999"]
SOAK_HASH_INTERVAL = 64
SOAK_SMOKE_FRAMES = 20000
# sys_ticrate default (Quake/host.c:79) -- the real-time pacing floor per
# frame for BOTH processes: the dedicated server always, and since the
# Quake/main_sdl.c fix that landed with this gate, the harness client too
# (harness_active && !harness_fixed_dt). Every timeout below is sized from
# it rather than from a flat constant.
SYS_TICRATE_DEFAULT = 0.025
SOAK_FULL_FRAMES = 100000
# The injected fault must be one this gate can actually observe. A pure
# simulation skew (the original sv_gravity 400) is invisible here: since the
# gate stopped being hash-exact (see compare_soak_hashes), what it watches is
# session liveness and the client's packet profile, and gravity changes
# neither -- the server still emits one unreliable datagram per frame whatever
# the entities are doing. net_messagetimeout 0 makes the server's own
# net_dgrm.c:204 timeout check fire on the next frame and drop the connected
# client, which the gate sees as a truncated hash stream (missing checkpoints)
# and/or a collapsed packet profile. Still a config skew, never a byte hack.
SOAK_INJECT_CVAR = "net_messagetimeout"
SOAK_INJECT_VALUE = "0"
SOAK_RECORD_NAME = "soak_repro"
# `record` is scripted at frame 0 BEFORE `connect` (same frame, file order
# preserved -- see Harness_LoadCmds' stable sort), not after: CL_Record_f
# (cl_demo.c) only allows starting a recording mid-session (c==2 &&
# cls.state==ca_connected) for cl.protocol in {NETQUAKE, FITZQUAKE, RMQ} --
# FTE+999 is rejected with "protocol not supported for recording mid-map".
# Recording before connecting takes the unconditional pre-connect path
# instead, which is not protocol-gated.


def build_combos(vkquake_c, vkquake_rs):
    """The 4 server/client build combos shared by the matrix and the soak."""
    return [
        ("C/C", vkquake_c, vkquake_c),
        ("Csv/Rcl", vkquake_c, vkquake_rs),
        ("Rsv/Ccl", vkquake_rs, vkquake_c),
        ("R/R", vkquake_rs, vkquake_rs),
    ]


def _stage_entry(src, dst):
    try:
        os.symlink(src, dst)
    except OSError:
        shutil.copyfile(src, dst)


def stage(game_data):
    staging = tempfile.mkdtemp(prefix="vkq-interop-")
    for entry in sorted(os.listdir(game_data)):
        src = os.path.join(game_data, entry)
        if not os.path.isdir(src):
            continue
        dst = os.path.join(staging, entry)
        os.makedirs(dst, exist_ok=True)
        for f in sorted(os.listdir(src)):
            _stage_entry(os.path.join(src, f), os.path.join(dst, f))
    return staging


def summarize(path):
    counts = {}
    with open(path, "rb") as f:
        while True:
            hdr = f.read(7)
            if len(hdr) < 7:
                break
            (length,) = struct.unpack("<I", hdr[3:7])
            f.seek(length, os.SEEK_CUR)
            key = (hdr[0], hdr[2])
            counts[key] = counts.get(key, 0) + 1
    return counts


def wait_until_bound(host, port, timeout=20.0):
    """Block until the dedicated server has bound `port`, or time out.

    The engine's stdout is block-buffered when it is not a tty, so a
    readiness *marker* never appears until the process flushes at exit --
    polling its log cannot work. The bound socket itself is the signal:
    UDP4_OpenSocket/UDP6_OpenSocket never set SO_REUSEADDR, so once the
    server is listening our own bind of the same port fails EADDRINUSE.
    Returns True if the port was observed bound.

    The probe binds the wildcard address, not `127.0.0.1`/`::1`: the server
    binds `INADDR_ANY`/`in6addr_any`, and on Windows a second bind of a
    specific loopback address does NOT conflict with an existing wildcard
    bind (no EADDRINUSE) -- only two wildcard binds do. Linux/macOS treat
    both forms as conflicting, so the wildcard probe is correct everywhere.
    """
    v6 = host.startswith("[")
    family = socket.AF_INET6 if v6 else socket.AF_INET
    bind_host = "::" if v6 else "0.0.0.0"
    deadline = time.time() + timeout
    while time.time() < deadline:
        s = socket.socket(family, socket.SOCK_DGRAM)
        try:
            s.bind((bind_host, port))
        except OSError as e:
            if e.errno in (errno.EADDRINUSE, errno.EACCES):
                return True
            raise
        finally:
            s.close()
        time.sleep(0.1)
    return False


def run_cell(server_exe, client_exe, game_data, cell, host, port, frames, map_name):
    sv_dir = stage(game_data)
    cl_dir = stage(game_data)
    try:
        with open(os.path.join(sv_dir, "harness.cmds"), "w") as f:
            f.write(f"0 sv_protocol {cell}\n")
            f.write(f"0 map {map_name}\n")
        with open(os.path.join(cl_dir, "harness.cmds"), "w") as f:
            f.write(f"0 connect {host}:{port}\n")

        # NOT a PIPE: nothing here reads the server's stdout, and a
        # 600-frame session with a client attached overruns the 64 KB pipe
        # buffer, wedging the server in write() and failing the gate
        # spuriously. A file keeps the log for the early-exit diagnostic
        # below (libc flushes it at exit, which is exactly that case).
        sv_log = os.path.join(sv_dir, "server.log")
        with open(sv_log, "w") as logf:
            server = subprocess.Popen(
                [os.path.abspath(server_exe), "-dedicated", "-basedir", ".",
                 "-port", str(port), "-harnesscmds", "harness.cmds"],
                cwd=sv_dir, stdout=logf, stderr=subprocess.STDOUT, text=True)
        try:
            bound = wait_until_bound(host, port)
            if server.poll() is not None:
                tail = open(sv_log).read()[-1500:]
                return None, f"server exited early ({server.returncode}):\n{tail}"
            if not bound:
                return None, f"server never bound port {port}"
            time.sleep(0.5)  # let the frame-0 `map` command finish
            client = subprocess.run(
                [os.path.abspath(client_exe), "-headless", "-basedir", ".",
                 "-netcapture", "harness.cap",
                 "-exitafter", str(frames), "-harnesscmds", "harness.cmds"],
                cwd=cl_dir, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, timeout=max(600, int(frames * SYS_TICRATE_DEFAULT * 2) + 60))
            if client.returncode not in (0, 2):
                return None, f"client exited with {client.returncode}:\n" + client.stdout[-1500:]
        finally:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()

        out = client.stdout
        m = re.search(r"Using protocol (\S+)", out)
        proto = m.group(1) if m else None
        m = re.search(r"Harness: msgbadread=(\d+)", out)
        if not m:
            return None, "no msgbadread counter in client output:\n" + out[-1500:]
        badread = int(m.group(1))
        cap = os.path.join(cl_dir, "harness.cap")
        if not os.path.isfile(cap) or os.path.getsize(cap) == 0:
            return None, "no capture produced:\n" + out[-1500:]
        counts = summarize(cap)
        return {
            "proto": proto,
            "badread": badread,
            "recv_rel": counts.get((0, 1), 0),
            "recv_unrel": counts.get((0, 2), 0),
            "send_rel": counts.get((1, 1), 0),
            "send_unrel": counts.get((1, 2), 0),
        }, None
    finally:
        shutil.rmtree(sv_dir, ignore_errors=True)
        shutil.rmtree(cl_dir, ignore_errors=True)


_ERROR_RE = re.compile(r"(QUAKE ERROR:.*|Host_Error:.*)")


def _scan_for_error(*texts):
    """Find a Sys_Error/Host_Error marker in server log / client stdout text.

    Host_Error exits the process (and therefore the returncode check below
    already catches it) only on a dedicated server; on a client Host_Error
    longjmps back to the console instead of exiting, so returncode alone
    cannot detect it -- these text markers (both already printed by the
    existing C code, Sys_Error/Host_Error) are the only reliable signal.
    """
    for text in texts:
        if not text:
            continue
        idx = text.rfind("ERROR-OUT BEGIN")
        if idx != -1:
            return text[idx:idx + 1500]
        m = _ERROR_RE.search(text)
        if m:
            return m.group(0)
    return None


def parse_hashfile(path):
    """Parse a -demohash output file into ({frame: hex_hash}, end_tuple).

    Format (Harness_Frame / Harness_Shutdown, Quake/harness.c): one
    "F <framecount> <hex>" line per simulated frame, plus a final
    "END <framecount> <hex>" line at clean shutdown. end_tuple is
    (frame, hex) from the END line, or None if the file has no END line
    (e.g. the process was killed rather than exiting cleanly).
    """
    hashes = {}
    end = None
    with open(path) as f:
        for line in f:
            parts = line.split()
            if len(parts) != 3:
                continue
            tag, frame_s, h = parts
            try:
                frame = int(frame_s)
            except ValueError:
                continue
            if tag == "F":
                hashes[frame] = h
            elif tag == "END":
                end = (frame, h)
    return hashes, end


def run_soak_cell(server_exe, client_exe, game_data, protocol, host, port, frames,
                   map_name, inject_at=None, inject_cvar=SOAK_INJECT_CVAR,
                   inject_value=SOAK_INJECT_VALUE, record_demo=None):
    """Run one --soak cell: a dedicated server hashing its own simulation
    state every frame (-demohash, reusing Harness_HashServer -- the exact
    channel run_corpus.py's --check golden path already consumes) against a
    scripted headless client. Unlike run_cell, this does NOT clean up the
    staging directories -- the caller pulls failure artifacts out of
    sv_dir/cl_dir (in the returned dict) before deleting them itself.
    """
    sv_dir = stage(game_data)
    cl_dir = stage(game_data)
    sv_cmds = os.path.join(sv_dir, "harness.cmds")
    cl_cmds = os.path.join(cl_dir, "harness.cmds")
    sv_hashfile = os.path.join(sv_dir, "server_hash.txt")
    sv_log = os.path.join(sv_dir, "server.log")

    with open(sv_cmds, "w") as f:
        f.write(f"0 sv_protocol {protocol}\n")
        f.write(f"0 map {map_name}\n")
        if inject_at is not None:
            f.write(f"{inject_at} {inject_cvar} {inject_value}\n")

    with open(cl_cmds, "w") as f:
        if record_demo:
            f.write(f"0 record {record_demo}\n")
        f.write(f"0 connect {host}:{port}\n")

    result = {
        "ok": False, "error": None, "proto": None, "hashes": {}, "end": None,
        "sv_dir": sv_dir, "cl_dir": cl_dir, "sv_cmds": sv_cmds, "cl_cmds": cl_cmds,
        "sv_log": sv_log, "client_stdout": "", "demo_path": None, "traffic": None,
    }

    with open(sv_log, "w") as logf:
        server = subprocess.Popen(
            [os.path.abspath(server_exe), "-dedicated", "-basedir", ".",
             "-port", str(port), "-harnesscmds", "harness.cmds",
             "-demohash", "server_hash.txt", "-exitafter", str(frames)],
            cwd=sv_dir, stdout=logf, stderr=subprocess.STDOUT, text=True)
    try:
        bound = wait_until_bound(host, port)
        if server.poll() is not None:
            result["error"] = f"server exited early ({server.returncode}):\n" + open(sv_log).read()[-1500:]
            return result
        if not bound:
            result["error"] = f"server never bound port {port}"
            return result
        time.sleep(0.5)  # let the frame-0 `map` command finish

        try:
            client = subprocess.run(
                [os.path.abspath(client_exe), "-headless", "-basedir", ".",
                 "-netcapture", "harness.cap",
                 "-exitafter", str(frames), "-harnesscmds", "harness.cmds"],
                cwd=cl_dir, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, timeout=max(600, int(frames * SYS_TICRATE_DEFAULT * 2) + 60))
        except subprocess.TimeoutExpired as e:
            out = e.stdout
            result["client_stdout"] = out.decode("utf-8", "replace") if isinstance(out, (bytes, bytearray)) else (out or "")
            result["error"] = "client timed out"
            return result

        result["client_stdout"] = client.stdout or ""
        if client.returncode not in (0, 2):
            result["error"] = (_scan_for_error(client.stdout) or
                                f"client exited with {client.returncode}:\n" + client.stdout[-1500:])
            return result

        # -exitafter triggers Harness_Exit(2) from inside each process' own
        # frame loop. Both processes now pace to sys_ticrate in real time
        # (the server always; the client since the Quake/main_sdl.c fix that
        # landed with this gate), so they finish at roughly the same wall
        # clock -- the server merely trails by its ~0.5s head start plus
        # whatever pacing jitter accumulated. The remaining budget after the
        # client returns is therefore small, so keep a whole frames-worth of
        # slack rather than a flat margin: a short wait truncates the server
        # (Popen.terminate() reports rc=1 on Windows) before it reaches
        # -exitafter, which reads as a false "server exited with 1".
        try:
            server.wait(timeout=frames * SYS_TICRATE_DEFAULT + 60)
        except subprocess.TimeoutExpired:
            pass
    finally:
        if server.poll() is None:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()

    sv_log_text = open(sv_log).read()
    err = _scan_for_error(sv_log_text)
    if err:
        result["error"] = err
        return result
    if server.returncode not in (0, 2):
        result["error"] = f"server exited with {server.returncode}:\n{sv_log_text[-1500:]}"
        return result

    m = re.search(r"Using protocol (\S+)", result["client_stdout"])
    result["proto"] = m.group(1) if m else None

    if not os.path.isfile(sv_hashfile):
        result["error"] = "server produced no -demohash output"
        return result
    hashes, end = parse_hashfile(sv_hashfile)
    if not hashes:
        result["error"] = "server hash file is empty"
        return result
    result["hashes"] = hashes
    result["end"] = end

    cap = os.path.join(cl_dir, "harness.cap")
    if not os.path.isfile(cap) or os.path.getsize(cap) == 0:
        result["error"] = "client produced no -netcapture output"
        return result
    m = re.search(r"Harness: msgbadread=(\d+)", result["client_stdout"])
    if not m:
        result["error"] = ("no msgbadread counter in client output:\n"
                           + result["client_stdout"][-1500:])
        return result
    counts = summarize(cap)
    result["traffic"] = {
        "badread": int(m.group(1)),
        "recv_rel": counts.get((0, 1), 0),
        "recv_unrel": counts.get((0, 2), 0),
        "send_rel": counts.get((1, 1), 0),
        "send_unrel": counts.get((1, 2), 0),
    }

    if record_demo:
        demo_path = os.path.join(cl_dir, "id1", f"{record_demo}.dem")
        if os.path.isfile(demo_path):
            result["demo_path"] = demo_path

    result["ok"] = True
    return result


def compare_soak_hashes(reference, cell_hashes, hash_interval):
    """Liveness check over a cell's hash stream, plus a *diagnostic* report of
    where it diverges from the reference.

    Only a missing checkpoint is a failure: the reference reached that frame
    and the cell did not, which means the cell's server died, dropped, or
    exited early -- a genuine soak failure.

    Hash *inequality* is deliberately NOT a failure. See the module
    docstring's "Why this gate is not hash-exact" section: the server's own
    simulation is fixed-dt and deterministic given its input, but which
    server frame each live UDP datagram from an independently-launched
    client lands on is not, and a one-frame shift in when a clc_move is
    applied permanently forks two otherwise-identical simulations. Measured
    on this machine with identical binaries, zero scripted input and no
    injected fault, two clean C/C sessions fork at ~frame 128 and never
    reconverge. The onset frame is still recorded because a *large* shift in
    it between builds is a useful smell, but it cannot be a pass/fail
    criterion.

    Returns (ok, detail) where detail has "kind" in {None, "drop"} and
    carries "diverged_at" (first mismatching checkpoint, or None).
    """
    checkpoints = sorted(f for f in reference if f % hash_interval == 0)
    diverged_at = None
    for frame in checkpoints:
        cell_hash = cell_hashes.get(frame)
        if cell_hash is None:
            highest = max(cell_hashes) if cell_hashes else -1
            return False, {
                "kind": "drop", "frame": frame, "diverged_at": diverged_at,
                "detail": f"cell has no hash for checkpoint frame {frame} "
                          f"(highest cell frame reached: {highest})",
            }
        if diverged_at is None and cell_hash != reference[frame]:
            diverged_at = frame
    return True, {"kind": None, "frame": None, "diverged_at": diverged_at,
                  "detail": None}


def compare_soak_traffic(reference, cell):
    """Compare a soak cell's client-side packet profile against the
    reference's, using exactly the tolerance model the non-soak matrix gate
    already uses (see main()): reliable counts must match exactly, unreliable
    counts get a +-max(6, 10%) live-timing noise band, and msgbadread is
    judged by its offset from the received-message count rather than its raw
    value. Returns (ok, detail_or_None).
    """
    for k in ("recv_rel", "send_rel"):
        if cell[k] != reference[k]:
            return False, (f"reliable traffic differs: {k}={cell[k]} "
                           f"vs reference {reference[k]}")
    for k in ("recv_unrel", "send_unrel"):
        base = reference[k]
        lo, hi = base - max(6, base // 10), base + max(6, base // 10)
        if not lo <= cell[k] <= hi:
            return False, (f"{k} outside noise floor: {cell[k]} vs "
                           f"reference {base} (allowed {lo}..{hi})")
    delta = cell["badread"] - cell["recv_rel"] - cell["recv_unrel"]
    bdelta = reference["badread"] - reference["recv_rel"] - reference["recv_unrel"]
    if delta != bdelta:
        return False, (f"msgbadread profile differs: offset {delta} vs "
                       f"reference {bdelta}")
    return True, None


def dump_soak_failure(results_dir, label, protocol, combo_name, reference, cell_result, mismatch):
    """Write the failure artifacts required by the soak spec: a bounded
    hash-window from both streams around the failing frame, the failing
    frame index, the server log, client stdout, both harness.cmds scripts,
    and the client-side repro demo (if the client survived long enough to
    record one) -- there is no server-side demo mechanism in this engine.
    """
    out_dir = os.path.join(results_dir, f"{protocol}_{combo_name.replace('/', '-')}")
    os.makedirs(out_dir, exist_ok=True)

    frame = mismatch.get("frame")
    window = 20  # checkpoints of context on each side of the failure

    def write_window(name, chain):
        checkpoints = sorted(f for f in chain if f % SOAK_HASH_INTERVAL == 0)
        idx = checkpoints.index(frame) if frame in checkpoints else len(checkpoints)
        lo, hi = max(0, idx - window), min(len(checkpoints), idx + window + 1)
        with open(os.path.join(out_dir, name), "w") as f:
            for cp in checkpoints[lo:hi]:
                marker = "  <-- FAIL" if cp == frame else ""
                f.write(f"{cp} {chain.get(cp, '<missing>')}{marker}\n")

    write_window("reference-hashes.txt", reference)
    write_window("cell-hashes.txt", cell_result.get("hashes") or {})

    with open(os.path.join(out_dir, "FAILURE.txt"), "w") as f:
        f.write(f"cell: {label}\n")
        f.write(f"kind: {mismatch.get('kind')}\n")
        f.write(f"failing frame: {frame}\n")
        f.write(f"detail: {mismatch.get('detail')}\n")
        if cell_result.get("error"):
            f.write(f"error: {cell_result['error']}\n")

    for src_key, dst_name in (("sv_cmds", "server-harness.cmds"), ("cl_cmds", "client-harness.cmds"),
                               ("sv_log", "server.log")):
        src = cell_result.get(src_key)
        if src and os.path.isfile(src):
            shutil.copyfile(src, os.path.join(out_dir, dst_name))
    if cell_result.get("client_stdout"):
        with open(os.path.join(out_dir, "client-stdout.txt"), "w") as f:
            f.write(cell_result["client_stdout"])
    demo = cell_result.get("demo_path")
    if demo and os.path.isfile(demo):
        shutil.copyfile(demo, os.path.join(out_dir, os.path.basename(demo)))

    return out_dir


def run_soak(args):
    protocols = args.protocols
    combos = build_combos(args.vkquake_c, args.vkquake_rs)
    if args.combos:
        combos = [c for c in combos if c[0] in args.combos]

    if args.list:
        total = len(protocols) * len(combos)
        print(f"soak plan: {len(protocols)} protocol(s) x {len(combos)} combo(s) "
              f"= {total} cell(s) + {len(protocols)} reference(s), "
              f"frames={args.frames}, hash-interval={args.hash_interval}")
        port = args.base_port
        for proto in protocols:
            print(f"  reference   C/C      @ {proto:9s} port={port}")
            port += 1
            for name, _sv, _cl in combos:
                print(f"  {proto:9s}  {name:8s} port={port}")
                port += 1
        return True

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA")

    host = "127.0.0.1"
    port = args.base_port

    # A separate C/C reference is generated PER PROTOCOL, not once overall:
    # even a fully idle client (no scripted input beyond `connect`) produces
    # a deterministic server state-hash divergence between protocols by
    # frame 128 (verified empirically -- Base-666 vs FTE+999 on an identical
    # C binary/build/map diverge with zero player input). The negotiated
    # wire protocol is NOT hash-invisible, so reusing one protocol's
    # reference for another is a false-positive generator, not a valid gate.
    references = {}
    for proto in protocols:
        print(f"soak: generating C/C reference stream @ {proto}, {args.frames} frames...")
        ref = run_soak_cell(args.vkquake_c, args.vkquake_c, args.game_data, proto,
                             host, port, args.frames, args.map)
        port += 1
        if not ref["ok"]:
            print(f"FAIL reference C/C @ {proto}: {ref['error']}")
            shutil.rmtree(ref["sv_dir"], ignore_errors=True)
            shutil.rmtree(ref["cl_dir"], ignore_errors=True)
            return False
        references[proto] = {"hashes": ref["hashes"], "traffic": ref["traffic"]}
        ref_checkpoints = sorted(f for f in ref["hashes"] if f % args.hash_interval == 0)
        # Minimum-checkpoint floor, same idea as trace_diff.py's/builtin_diff.py's
        # minimum-record floors. Every cell's liveness bar is derived from this
        # stream, so a truncated reference would lower the bar to itself and pass
        # the whole protocol vacuously -- the same shape as the C/C short-circuit
        # this gate removed, one level up.
        expected_checkpoints = args.frames // args.hash_interval
        if len(ref_checkpoints) < expected_checkpoints:
            print(f"FAIL reference C/C @ {proto}: reference stream is short -- "
                  f"{len(ref_checkpoints)} checkpoints, expected >= {expected_checkpoints}; "
                  f"every cell's liveness bar derives from this stream, so a truncated "
                  f"reference would pass this protocol vacuously")
            shutil.rmtree(ref["sv_dir"], ignore_errors=True)
            shutil.rmtree(ref["cl_dir"], ignore_errors=True)
            return False
        t = ref["traffic"]
        print(f"  reference: {len(ref['hashes'])} frames hashed, {len(ref_checkpoints)} checkpoints, "
              f"rel {t['recv_rel']}/{t['send_rel']} unrel {t['recv_unrel']}/{t['send_unrel']}")
        shutil.rmtree(ref["sv_dir"], ignore_errors=True)
        shutil.rmtree(ref["cl_dir"], ignore_errors=True)

    ok = True
    rows = []
    for proto in protocols:
        reference = references[proto]["hashes"]
        ref_traffic = references[proto]["traffic"]
        ref_checkpoints = sorted(f for f in reference if f % args.hash_interval == 0)
        for name, sv, cl in combos:
            # C/C is deliberately NOT short-circuited as "== its own
            # reference". Doing so is what hid two real engine bugs
            # (Quake/main_sdl.c client pacing, Quake/net_wins.c
            # WSAECONNRESET) for the whole of M1: the comparison path had
            # literally never executed against two independently launched
            # processes. C/C re-runs like any other combo, and its cost is
            # the price of the gate being real.
            label = f"{proto} {name}"
            r = run_soak_cell(sv, cl, args.game_data, proto, host, port, args.frames, args.map,
                               inject_at=args.inject_desync_at, inject_cvar=args.inject_cvar,
                               inject_value=args.inject_value, record_demo=SOAK_RECORD_NAME)
            port += 1

            cell_ok = r["ok"]
            note = ""
            mismatch = {"kind": None, "frame": None, "diverged_at": None, "detail": None}
            if cell_ok:
                expect = EXPECT_PROTO.get(proto)
                if r["proto"] != expect:
                    cell_ok = False
                    note = f"negotiated {r['proto']!r}, expected {expect!r}"
            if cell_ok:
                cmp_ok, mismatch = compare_soak_hashes(reference, r["hashes"], args.hash_interval)
                if not cmp_ok:
                    cell_ok = False
                    note = mismatch["detail"]
            if cell_ok:
                traffic_ok, traffic_note = compare_soak_traffic(ref_traffic, r["traffic"])
                if not traffic_ok:
                    cell_ok = False
                    note = traffic_note

            if not cell_ok:
                ok = False
                if not note:
                    note = r["error"] or "unknown failure"
                out_dir = dump_soak_failure(args.results_dir, label, proto, name, reference, r, mismatch)
                print(f"  {label:16s} FAIL  {note}")
                print(f"    artifacts: {out_dir}")
                rows.append((label, "FAIL", note))
            else:
                # hash divergence is reported, never fatal -- see
                # compare_soak_hashes' docstring for why
                fork = mismatch.get("diverged_at")
                diag = "hash-identical" if fork is None else f"hash forks @{fork} (diagnostic)"
                print(f"  {label:16s} PASS  proto={r['proto']:7s} "
                      f"{len(ref_checkpoints)} checkpoints reached, traffic in band, {diag}")
                rows.append((label, "PASS", f"{len(ref_checkpoints)} checkpoints, {diag}"))

            shutil.rmtree(r["sv_dir"], ignore_errors=True)
            shutil.rmtree(r["cl_dir"], ignore_errors=True)

    print()
    print(f"{'cell':22s} {'result':6s} detail")
    for label, status, note in rows:
        print(f"{label:22s} {status:6s} {note}")
    print("interop_matrix --soak:", "PASS" if ok else "FAIL")
    return ok


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake-c", required=True)
    p.add_argument("--vkquake-rs", required=True)
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--frames", type=int, default=None,
                   help="default: 600 (matrix), or 20000/--soak smoke (100000 for a full soak)")
    p.add_argument("--map", default="start")
    p.add_argument("--cells", nargs="*", default=CELLS)
    p.add_argument("--ipv6", action="store_true",
                   help="also run the FTE+999 cell over [::1] (local-only leg)")
    p.add_argument("--base-port", type=int, default=26100)
    p.add_argument("--list", action="store_true",
                   help="print the planned run matrix/soak cells and exit without executing anything")
    # --soak (Phase 7 M1 T1.3): see the module docstring's "--soak:" section.
    p.add_argument("--soak", action="store_true",
                   help="run the netplay-soak gate instead of the 4-way protocol matrix")
    p.add_argument("--protocols", nargs="*", default=None,
                   help="--soak only; defaults to Base-666 and FTE+999")
    p.add_argument("--combos", nargs="*", default=None,
                   choices=["C/C", "Csv/Rcl", "Rsv/Ccl", "R/R"],
                   help="--soak only; restrict to a subset of the 4 build combos")
    p.add_argument("--hash-interval", type=int, default=SOAK_HASH_INTERVAL,
                   help="--soak only; compare state hashes every N server frames")
    p.add_argument("--results-dir", default=None,
                   help="--soak only; failure-artifact directory (default: ./soak-results)")
    # Fault-injection proof hook -- hidden from --help on purpose (it is not
    # a normal gate flag, it exists so the orchestrator can prove the
    # detector fires; see the module docstring's "Fault-injection proof
    # hook" paragraph for exactly what it does).
    p.add_argument("--inject-desync-at", type=int, default=None, help=argparse.SUPPRESS)
    p.add_argument("--inject-cvar", default=SOAK_INJECT_CVAR, help=argparse.SUPPRESS)
    p.add_argument("--inject-value", default=SOAK_INJECT_VALUE, help=argparse.SUPPRESS)
    args = p.parse_args()

    if args.soak:
        if args.frames is None:
            args.frames = SOAK_SMOKE_FRAMES
        if args.protocols is None:
            args.protocols = SOAK_PROTOCOLS
        else:
            bad = [proto for proto in args.protocols if proto not in EXPECT_PROTO]
            if bad:
                sys.exit(f"error: unknown --protocols entries: {bad}")
        if args.results_dir is None:
            args.results_dir = os.path.join(os.getcwd(), "soak-results")
        ok = run_soak(args)
        sys.exit(0 if ok else 1)

    if args.frames is None:
        args.frames = 600

    combos = build_combos(args.vkquake_c, args.vkquake_rs)

    jobs = [(cell, "127.0.0.1") for cell in args.cells]
    if args.ipv6:
        jobs.append(("FTE+999", "[::1]"))

    if args.list:
        print(f"matrix plan: {len(jobs)} cell(s) x {len(combos)} combo(s) "
              f"= {len(jobs) * len(combos)} run(s), frames={args.frames}")
        port = args.base_port
        for cell, host in jobs:
            for name, _sv, _cl in combos:
                print(f"  {cell:9s}@{host:9s} {name:8s} port={port}")
                port += 1
        return

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA")

    ok = True
    port = args.base_port
    for cell, host in jobs:
        results = {}
        label = f"{cell}@{host}"
        for name, sv, cl in combos:
            r, err = run_cell(sv, cl, args.game_data, cell, host, port,
                              args.frames, args.map)
            port += 1
            if err:
                print(f"FAIL {label} {name}: {err}")
                ok = False
                continue
            results[name] = r
            expect = EXPECT_PROTO[cell]
            if r["proto"] != expect:
                print(f"FAIL {label} {name}: negotiated {r['proto']!r}, expected {expect!r}")
                ok = False
            # absolute floor: signon reliables must have flowed; unreliable
            # volume is protocol-dependent (a Base-15 idle session emits
            # almost none) so its health is judged against the C/C baseline
            # below, not an absolute number
            if r["recv_rel"] < 2:
                print(f"FAIL {label} {name}: unhealthy traffic {r}")
                ok = False
            print(f"  {label:16s} {name:8s} proto={r['proto']:7s} "
                  f"rel {r['recv_rel']}/{r['send_rel']} "
                  f"unrel {r['recv_unrel']}/{r['send_unrel']} "
                  f"badread={r['badread']}")

        if len(results) == 4:
            base = results["C/C"]
            for name, r in results.items():
                if r["recv_rel"] != base["recv_rel"] or r["send_rel"] != base["send_rel"]:
                    print(f"FAIL {label}: reliable counts differ: {name} "
                          f"{r['recv_rel']}/{r['send_rel']} vs C/C "
                          f"{base['recv_rel']}/{base['send_rel']}")
                    ok = False
                for k in ("recv_unrel", "send_unrel"):
                    lo = base[k] - max(6, base[k] // 10)
                    hi = base[k] + max(6, base[k] // 10)
                    if not lo <= r[k] <= hi:
                        print(f"FAIL {label}: {k} outside noise floor: "
                              f"{name} {r[k]} vs C/C {base[k]}")
                        ok = False
                # msgbadread scales with the number of received messages
                # (the dgrm/parse paths probe optional fields per message),
                # so live +-1-message timing noise shifts the raw counter;
                # the per-session invariant is its offset from the received
                # message count
                delta = r["badread"] - r["recv_rel"] - r["recv_unrel"]
                bdelta = base["badread"] - base["recv_rel"] - base["recv_unrel"]
                if delta != bdelta:
                    print(f"FAIL {label}: msgbadread profile differs: {name} "
                          f"{r['badread']}-{r['recv_rel'] + r['recv_unrel']} "
                          f"vs C/C {base['badread']}-"
                          f"{base['recv_rel'] + base['recv_unrel']}")
                    ok = False

    print("interop_matrix:", "PASS" if ok else "FAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
