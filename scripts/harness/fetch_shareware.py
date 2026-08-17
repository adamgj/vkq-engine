#!/usr/bin/env python3
"""Fetch the freely redistributable Quake 1.06 shareware data for CI.

Downloads quake106.zip (sha256-pinned) from a mirror list, extracts
resource.1 (an LHA self-extracting archive) with lhasa/lha/7z, and leaves
id1/pak0.pak under the output directory, which then works as
QUAKE_GAME_DATA for the harness scripts.

The shareware episode may be freely distributed per its LICINFO.TXT; the
registered/mission-pack corpus tiers are local-only (see Misc/harness).
"""

import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.request

MIRRORS = [
    "https://ftp.gwdg.de/pub/misc/ftp.idsoftware.com/idstuff/quake/quake106.zip",
    "https://www.gamers.org/pub/idgames2/idstuff/quake/quake106.zip",
]
ZIP_SHA256 = "ec6c9d34b1ae0252ac0066045b6611a7919c2a0d78a3a66d9387a8f597553239"
PAK0_SHA256 = "35a9c55e5e5a284a159ad2a62e0e8def23d829561fe2f54eb402dbc0a9a946af"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def find_extractor():
    for tool, argv in [("lha", ["lha", "xqw="]), ("lhasa", None), ("7z", ["7z", "x", "-y"]), ("7zz", ["7zz", "x", "-y"])]:
        if shutil.which(tool):
            return tool
    return None


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "shareware-data"
    pak_dst = os.path.join(out, "id1", "pak0.pak")
    if os.path.isfile(pak_dst) and sha256(pak_dst) == PAK0_SHA256:
        print(f"already present: {pak_dst}")
        return

    tool = find_extractor()
    if not tool:
        sys.exit("error: need lha, lhasa, 7z or 7zz on PATH to extract resource.1\n"
                 "  ubuntu: apt install lhasa   macOS: brew install lhasa   windows: 7z is preinstalled")

    tmp = tempfile.mkdtemp(prefix="quake106-")
    zip_path = os.path.join(tmp, "quake106.zip")
    for url in MIRRORS:
        try:
            print(f"downloading {url}")
            urllib.request.urlretrieve(url, zip_path)
            if sha256(zip_path) == ZIP_SHA256:
                break
            print("checksum mismatch, trying next mirror")
        except Exception as e:  # noqa: BLE001 - try the next mirror on any failure
            print(f"failed: {e}")
    else:
        sys.exit("error: could not fetch quake106.zip with the pinned checksum from any mirror")

    import zipfile
    with zipfile.ZipFile(zip_path) as z:
        z.extract("resource.1", tmp)

    if tool in ("lha", "lhasa"):
        cmd = [tool, "xqw=" + tmp, os.path.join(tmp, "resource.1")]
        # lhasa's lha emulation: extract into tmp via cwd instead of w= if needed
        proc = subprocess.run(cmd, cwd=tmp)
        if proc.returncode != 0:
            proc = subprocess.run([tool, "xq", "resource.1"], cwd=tmp)
            if proc.returncode != 0:
                sys.exit("error: lha extraction failed")
    else:
        proc = subprocess.run([tool, "x", "-y", "resource.1"], cwd=tmp)
        if proc.returncode != 0:
            sys.exit("error: 7z extraction failed")

    pak_src = None
    for root, _dirs, files in os.walk(tmp):
        for f in files:
            if f.lower() == "pak0.pak":
                pak_src = os.path.join(root, f)
    if not pak_src:
        sys.exit("error: pak0.pak not found in extracted shareware")
    if sha256(pak_src) != PAK0_SHA256:
        sys.exit("error: extracted pak0.pak has unexpected checksum")

    os.makedirs(os.path.dirname(pak_dst), exist_ok=True)
    shutil.copyfile(pak_src, pak_dst)
    shutil.rmtree(tmp, ignore_errors=True)
    print(f"ready: {pak_dst}")


if __name__ == "__main__":
    main()
