#!/usr/bin/env python3
"""
get_config.py - Extract DBS distribution configuration from TOML files.

Parses a DBS configuration file (e.g. tacos-distro.toml, tacos.toml) and prints
shell variable assignments suitable for `eval` in Bash scripts.
"""

import sys
import os
import shlex

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib  # Fallback for Python < 3.11 if tomli is installed
    except ImportError:
        tomllib = None


# Mapping from TOML [distro] keys to Bash environment variable names
DISTRO_KEY_MAPPING = {
    "name": "DISTRO_NAME",
    "arch": "ARCH",
    "dest": "DISTRO_ROOT",
    "staging_dir": "STAGING_DIR",
    "sign_key": "GPG_KEY",
    "base_url": "BASE_URL",
    "workers": "WORKERS",
}


def parse_toml_config(config_path: str, section: str = "distro") -> dict:
    """Reads and parses the specified section from a TOML configuration file."""
    if not os.path.isfile(config_path):
        return {}

    if tomllib is not None:
        try:
            with open(config_path, "rb") as f:
                data = tomllib.load(f)
            return data.get(section, {})
        except Exception as e:
            sys.stderr.write(f"Warning: Failed to parse {config_path}: {e}\n")
            return {}

    # Basic fallback line parser if tomllib is unavailable
    result = {}
    in_section = False
    with open(config_path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("[") and line.endswith("]"):
                in_section = (line[1:-1].strip() == section)
                continue
            if in_section and "=" in line:
                k, v = line.split("=", 1)
                k = k.strip()
                v = v.strip().strip("\"'")
                result[k] = v
    return result


def main():
    if len(sys.argv) > 1 and sys.argv[1] in ("-h", "--help"):
        print("Usage: get_config.py [CONFIG_FILE] [SECTION]")
        print("Extracts [distro] section keys as shell assignments for eval.")
        sys.exit(0)

    config_path = sys.argv[1] if len(sys.argv) > 1 else "tacos-distro.toml"
    section = sys.argv[2] if len(sys.argv) > 2 else "distro"

    distro_cfg = parse_toml_config(config_path, section=section)

    for toml_key, bash_var in DISTRO_KEY_MAPPING.items():
        if toml_key in distro_cfg:
            val = distro_cfg[toml_key]
            # Use shlex.quote for safe shell consumption
            print(f"{bash_var}={shlex.quote(str(val))}")


if __name__ == "__main__":
    main()
