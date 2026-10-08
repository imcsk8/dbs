#!/usr/bin/env python3
"""
check_rpm_signature.py - Fast RPM header signature validator and scanner.

Inspects the OpenPGP/RSA signature tags in the RPM lead and signature header:
  - RPMTAG_SIGGPG (1002)
  - RPMTAG_SIGPGP (1005)
  - RPMTAG_RSAHEADER (268)

Can be used to:
  1. Check a single RPM file (returns exit code 0 if signed, 1 if unsigned).
  2. Scan a directory of RPMs and stream null-terminated paths of unsigned RPMs
     for high-performance parallel signing with `xargs -0 rpmsign`.
"""

import argparse
import os
import struct
import sys


def is_signed_rpm(path: str) -> bool:
    """
    Inspects the RPM file signature header to check if an OpenPGP/RSA signature exists.
    Returns True if signed, False if unsigned or unreadable.
    """
    try:
        with open(path, "rb") as f:
            magic = f.read(4)
            if magic != b"\xed\xab\xee\xdb":
                return False
            f.seek(96)
            smagic = f.read(3)
            if smagic != b"\x8e\xad\xe8":
                return False
            f.seek(96 + 8)
            nindex, _ = struct.unpack(">II", f.read(8))
            for _ in range(nindex):
                tag = struct.unpack(">I", f.read(16)[:4])[0]
                if tag in (1002, 1005, 268):
                    return True
            return False
    except Exception:
        return False


def is_unsigned_rpm(path: str) -> bool:
    """Returns True if the RPM exists and does not contain a signature."""
    try:
        with open(path, "rb") as f:
            magic = f.read(4)
            if magic != b"\xed\xab\xee\xdb":
                return False
            f.seek(96)
            smagic = f.read(3)
            if smagic != b"\x8e\xad\xe8":
                return False
            f.seek(96 + 8)
            nindex, _ = struct.unpack(">II", f.read(8))
            for _ in range(nindex):
                tag = struct.unpack(">I", f.read(16)[:4])[0]
                if tag in (1002, 1005, 268):
                    return False
            return True
    except Exception:
        return False


def scan_directory(dir_path: str, find_unsigned: bool = True, null_delim: bool = True, count_only: bool = False):
    """Scans a directory for RPMs and outputs matching file paths or counts."""
    count = 0
    if not os.path.isdir(dir_path):
        if count_only:
            print(0)
        return

    filter_fn = is_unsigned_rpm if find_unsigned else is_signed_rpm

    for entry in os.scandir(dir_path):
        if entry.is_file() and entry.name.endswith(".rpm"):
            if filter_fn(entry.path):
                count += 1
                if not count_only:
                    if null_delim:
                        sys.stdout.buffer.write(entry.path.encode("utf-8") + b"\0")
                    else:
                        print(entry.path)

    if count_only:
        print(count)


def main():
    parser = argparse.ArgumentParser(
        description="Fast RPM header signature validator and scanner."
    )
    parser.add_argument(
        "path",
        help="Path to an RPM file or directory containing RPMs to inspect."
    )
    parser.add_argument(
        "-u", "--unsigned",
        action="store_true",
        default=True,
        help="Find unsigned RPMs (default behavior for directories)."
    )
    parser.add_argument(
        "-s", "--signed",
        action="store_true",
        help="Find signed RPMs instead of unsigned."
    )
    parser.add_argument(
        "-0", "--null",
        action="store_true",
        default=False,
        help="Output null-terminated strings (\0) suitable for xargs -0."
    )
    parser.add_argument(
        "-c", "--count",
        action="store_true",
        help="Output only the count of matching RPMs."
    )
    parser.add_argument(
        "-q", "--quiet",
        action="store_true",
        help="Quiet mode: exit 0 if signed, exit 1 if unsigned (single file mode)."
    )

    args = parser.parse_args()
    find_unsigned = not args.signed

    target_path = args.path

    if os.path.isdir(target_path):
        # In directory mode, default to null delimiter if not counting and not specified otherwise
        null_delim = args.null or (not args.count and not sys.stdout.isatty())
        if args.null:
            null_delim = True
        scan_directory(
            target_path,
            find_unsigned=find_unsigned,
            null_delim=null_delim,
            count_only=args.count
        )
    elif os.path.isfile(target_path):
        signed = is_signed_rpm(target_path)
        if args.quiet:
            sys.exit(0 if signed else 1)
        if args.null:
            matches = (not signed) if find_unsigned else signed
            if matches:
                sys.stdout.buffer.write(target_path.encode("utf-8") + b"\0")
        else:
            status = "SIGNED" if signed else "UNSIGNED"
            print(f"{target_path}: {status}")
            sys.exit(0 if signed else 1)
    else:
        sys.stderr.write(f"Error: Path not found: {target_path}\n")
        sys.exit(2)


if __name__ == "__main__":
    main()
