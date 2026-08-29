#!/usr/bin/env python3
"""Generate a minimal but structurally valid PE32+ (x64) executable whose only
content is a .rsrc section carrying a well-formed VS_VERSIONINFO resource.

Purpose: feed NSIS `!getdllversion` at compile time in the nsis-hook compile
harness on machines that have no Windows toolchain (macOS/Linux CI).
The file is never executed; only its headers and resource tree are parsed.

Usage: make_versioned_pe.py --version 0.14.28.0 --out path/to/fake.exe [--pad BYTES]
"""
import argparse
import struct


def build_versioninfo(ms: int, ls: int) -> bytes:
    key = "VS_VERSION_INFO".encode("utf-16-le") + b"\x00\x00"
    fixed = struct.pack(
        "<13I",
        0xFEEF04BD,  # dwSignature
        0x00010000,  # dwStrucVersion
        ms,          # dwFileVersionMS
        ls,          # dwFileVersionLS
        ms,          # dwProductVersionMS
        ls,          # dwProductVersionLS
        0x3F,        # dwFileFlagsMask
        0,           # dwFileFlags
        0x00040004,  # dwFileOS (VOS_NT_WINDOWS32)
        1,           # dwFileType (VFT_APP)
        0,           # dwFileSubtype
        0,           # dwFileDateMS
        0,           # dwFileDateLS
    )
    # header: wLength wValueLength wType + szKey + padding to 32-bit
    hdr_no_len = struct.pack("<HH", 52, 0) + key
    pad = (4 - ((2 + len(hdr_no_len)) % 4)) % 4
    body = hdr_no_len + b"\x00" * pad + fixed
    total = 2 + len(body)
    return struct.pack("<H", total) + body


def build_rsrc(version_data: bytes):
    # layout (offsets within section):
    # 0x00 root dir (16) + 1 entry (8)      -> RT_VERSION (16)
    # 0x18 dir2     (16) + 1 entry (8)      -> ID 1
    # 0x30 dir3     (16) + 1 entry (8)      -> lang 1033
    # 0x48 data entry (16)
    # 0x58 version data
    def directory(n_ids):
        return struct.pack("<IIHHHH", 0, 0, 0, 0, 0, n_ids)

    data_entry_off = 0x48
    data_off = 0x58
    root = directory(1) + struct.pack("<II", 16, 0x80000000 | 0x18)
    dir2 = directory(1) + struct.pack("<II", 1, 0x80000000 | 0x30)
    dir3 = directory(1) + struct.pack("<II", 0x0409, data_entry_off)
    # data entry: OffsetToData is an RVA (section VA = 0x1000)
    dent = struct.pack("<IIII", 0x1000 + data_off, len(version_data), 0, 0)
    blob = root + dir2 + dir3 + dent
    assert len(blob) == data_off, len(blob)
    return blob + version_data


def build_pe(ms: int, ls: int, pad_to: int, with_version: bool = True) -> bytes:
    rsrc = build_rsrc(build_versioninfo(ms, ls))
    file_align = 0x200
    sect_align = 0x1000
    rsrc_raw_size = (len(rsrc) + file_align - 1) // file_align * file_align

    dos = b"MZ" + b"\x00" * 58 + struct.pack("<I", 0x40)
    coff = struct.pack("<HHIIIHH", 0x8664, 1, 0, 0, 0, 0xF0, 0x0022)
    ddirs = [(0, 0)] * 16
    if with_version:
        ddirs[2] = (sect_align, len(rsrc))  # resource directory
    # with_version=False: keep the section bytes (identical layout) but publish NO resource
    # directory - the exact shape of a cross-build/winresource regression (valid PE, no
    # discoverable VS_VERSIONINFO), which `!getdllversion` must fail to read.
    opt = struct.pack(
        "<HBBIIIIIQ", 0x20B, 14, 0, 0, len(rsrc), 0, 0, 0x1000, 0x140000000
    )
    opt += struct.pack(
        "<IIHHHHHHIIIIHHQQQQII",
        sect_align, file_align,
        6, 0,      # OS version
        0, 0,      # image version
        6, 0,      # subsystem version
        0,         # win32 version
        sect_align * 2,  # SizeOfImage (headers + one section page)
        file_align,      # SizeOfHeaders
        0,         # checksum
        3, 0x8160,  # subsystem CUI, dll characteristics
        0x100000, 0x1000, 0x100000, 0x1000,  # stack/heap
        0, 16,     # loader flags, number of rva+sizes
    )
    for va, sz in ddirs:
        opt += struct.pack("<II", va, sz)
    sect = struct.pack(
        "<8sIIIIIIHHI",
        b".rsrc\x00\x00\x00",
        len(rsrc), sect_align, rsrc_raw_size, file_align,
        0, 0, 0, 0, 0x40000040,
    )
    headers = dos + b"PE\x00\x00" + coff + opt + sect
    assert len(headers) <= file_align, len(headers)
    headers += b"\x00" * (file_align - len(headers))
    body = rsrc + b"\x00" * (rsrc_raw_size - len(rsrc))
    blob = headers + body
    if pad_to > len(blob):
        blob += b"\x00" * (pad_to - len(blob))
    return blob


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True, help="a.b.c.d")
    ap.add_argument("--out", required=True)
    ap.add_argument("--pad", type=int, default=0, help="pad file to at least N bytes")
    ap.add_argument(
        "--no-version",
        action="store_true",
        help="emit a valid PE with NO discoverable version resource (negative-control "
        "input: the blind-oracle guard must fail the hook compile on this file)",
    )
    args = ap.parse_args()
    parts = [int(x) for x in args.version.split(".")]
    while len(parts) < 4:
        parts.append(0)
    a, b, c, d = parts[:4]
    ms = (a << 16) | b
    ls = (c << 16) | d
    with open(args.out, "wb") as f:
        f.write(build_pe(ms, ls, args.pad, with_version=not args.no_version))
    tag = "NO VERSIONINFO" if args.no_version else f"version {a}.{b}.{c}.{d} MS={ms} LS={ls}"
    print(f"{args.out}: {tag}")


if __name__ == "__main__":
    main()
