# Distribution Build System (DBS)

A modern, high-throughput, distribution-agnostic operating system build platform written in Rust.

**DBS** eliminates opaque, ad-hoc distribution bootstrapping scripts ("secret sauces") by providing a unified, declarative pipeline for discovering, synchronizing, inspecting, analyzing, and compiling packages directly from **dist-git** repositories across any RPM-based distribution (Fedora Rawhide, CentOS Stream, TacOS, RHEL, AlmaLinux, Rocky, etc.).

---

## Key Features

* **Multi-Distribution Dist-Git Engine:** Pure Rust client supporting Fedora Rawhide (Pagure API), CentOS Stream 10/9 (GitLab API), TacOS (Forgejo/Codeberg API), and generic git repositories.
* **Upstream Re-branding & Remotes:** Clone upstream dist-git repositories under their original names or custom names (`--as`, `--rename-spec`), while reconfiguring remotes (`--new-origin`) so `origin` points to your distribution and `upstream` tracks upstream changes.
* **Topological DAG Compilation Engine:** Deterministic, non-recursive layered scheduler using **Kahn's algorithm (In-Degree BFS)** to resolve parallel compilation layers (Layer 0, Layer 1, ...) and immediately flag circular dependencies.
* **Hermetic Mock Runner:** Isolated build execution using Mock chroots with:
  * Native parallel worker pools (`-j`) via Tokio async semaphores.
  * Sequential Mock chain compilation (`--chain`).
  * Dynamic local repository feedback (`--dynamic-repo`), automatically indexing built RPMs and feeding them back to concurrent workers via `--addrepo=file://...`.
* **Content-Addressable Storage (CAS) & BTRFS Lookaside:** Native lookaside cache manager (`dbs lookaside`) storing source archives deduplicated by SHA-512 with Linux `FICLONE` Copy-on-Write (CoW) reflinks for instant, 0-disk-overhead source staging into Mock.
* **PostgreSQL Supply Chain Catalog:** Diesel-backed relational schema tracking operating systems, packages (EVR, source RPMs, git commits), granular capability dependencies (`Provides`, `Requires`, `BuildRequires`), and staged binary RPM artifacts.

---

## Quickstart

### 1. Prerequisites & Storage Setup

Ensure your host system has the required build and packaging utilities installed:
* **Rust & Cargo** (1.80+ or 2024 edition)
* **Mock** (v6.0+)
* **createrepo_c**
* **git**

```bash
# On Fedora / ELN / CentOS Stream:
sudo dnf install -y rust cargo mock createrepo_c git
sudo usermod -a -G mock $USER
newgrp mock
```

#### Recommended BTRFS Storage Setup (Optional for Production Builders)
DBS is filesystem-agnostic and works out-of-the-box on any Linux filesystem (Ext4, XFS, tmpfs, ZFS) by automatically falling back to hardlinks or standard file copies. 

For high-throughput builders, hosting `/srv/dbs/lookaside` on a dedicated **BTRFS subvolume** unlocks kernel-level Copy-on-Write (`FICLONE` ioctl) reflinks (instant 0.001s source staging with **0 extra bytes of disk space**) and transparent Zstandard compression:

```bash
# 1. Create dedicated lookaside subvolume
sudo mkdir -p /srv/dbs
sudo btrfs subvolume create /srv/dbs/lookaside

# 2. (Optional) Recommended mount options in /etc/fstab:
# UUID=<disk-uuid>  /srv/dbs/lookaside  btrfs  subvol=@lookaside,compress=zstd:3,noatime,space_cache=v2  0 0

# 3. Grant permissions to your user and mock group:
sudo chown -R $USER:mock /srv/dbs/lookaside
```

#### Recommended Host Kernel Tuning (High-Throughput Parallel Builds)
When running high-concurrency builds (`dbs dag -j4`, `dbs build`), multiple Mock / `systemd-nspawn` workers run simultaneously under the same host UID. Build tools (Cargo, make jobservers, Ninja) and comprehensive test suites (`strace`, `glibc`) create thousands of IPC pipes.

By default, Linux limits an unprivileged UID to 16,384 pipe pages (only 1,024 pipes at 64 KB each) via `fs.pipe-user-pages-soft`. Exceeding this limit causes Linux to **silently demote newly created pipes to 2 pages (8 KB)** and reject `fcntl(F_SETPIPE_SZ)` expansions, triggering jobserver deadlocks, throughput collapse, or test failures.

For dedicated build servers and CI nodes, install the provided host sysctl tuning profile:

```bash
sudo cp config/sysctl/99-dbs-build-host.conf /etc/sysctl.d/
sudo sysctl --system
```

### 2. Build DBS

Compile the project and install the binary:

```bash
git clone https://codeberg.org/imcsk8/dbs.git
cd dbs
make release
```

The optimized release binary is located at `./bin/dbs`. Verify installation:

```bash
./bin/dbs --help
```

### 3. Explore Remote Packages

Search for packages across Fedora Rawhide or CentOS Stream without leaving your terminal:

```bash
# Search Fedora Rawhide dist-git
./bin/dbs explore --distro fedora-rawhide --search zstd

# Search CentOS Stream 10 dist-git
./bin/dbs explore --distro centos-stream-10 --search python
```

### 4. Clone Dist-Git Repositories

Clone upstream package sources and configure your distribution's remote:

```bash
# Clone directly from Fedora Rawhide and configure Codeberg/Forgejo remote:
./bin/dbs distgit clone --distro fedora-rawhide zstd \
  --new-origin https://codeberg.org/imcsk8/tacos/zstd.git

# Clone under a custom package name with spec renaming:
./bin/dbs distgit clone --distro fedora-rawhide fedora-release \
  --as tacos-release --rename-spec
```

### 5. Analyze Dependencies & Compilation Layers (DAG)

Inspect package `.spec` files and calculate parallel compilation layers:

```bash
# Calculate build layers across dist-git packages
./bin/dbs dag -i data/distgit

# Generate a detailed Markdown dependency report
./bin/dbs dag -i data/distgit --report reports/dependency_layers.md
```

### 6. Build in Mock

Compile packages in isolated chroots:

```bash
# Compile a single package in Mock
./bin/dbs build -r fedora-rawhide-x86_64 -o staging data/distgit/zstd/zstd.spec

# Automatically compile all packages layer-by-layer in topological order
./bin/dbs dag -i data/distgit --build -r fedora-rawhide-x86_64 -j 4 -o staging
```

### 7. Manage Lookaside Cache (BTRFS CoW)

Store, verify, and maintain source tarballs with zero-disk BTRFS reflinks:

```bash
# Check lookaside status and filesystem engine
./bin/dbs lookaside status

# Upload and register a new source tarball and update dist-git 'sources' manifest:
./bin/dbs lookaside upload -p zstd -f /path/to/zstd-1.5.7.tar.gz --spec data/distgit/zstd/zstd.spec

# Pre-fetch and cache all sources across your cloned dist-git repositories:
./bin/dbs lookaside sync -i data/distgit -j 4

# Garbage collect unreferenced/orphaned source archives
./bin/dbs lookaside gc --dry-run
```

### 8. Build & Publish Complete Distribution Repositories

Automate end-to-end repository initialization, DAG compilation, repodata indexing, GPG signing, and web serving:

```bash
# Initialize a new distribution repository and Mock chroot profile
./bin/dbs distro init tacos-stable-x86_64 --arch x86_64 --channel stable

# Compile packages in topological DAG order and publish to the repository
./bin/dbs distro build tacos-stable-x86_64 --path /srv/dbs/tacos/rpm -j 4

# Check repository inventory, package counts, and repodata health
./bin/dbs distro status tacos-stable-x86_64

# Expose repository via embedded HTTP server or export Nginx config
./bin/dbs distro serve --path /srv/dbs/tacos/distro --port 8080
```

---

## Command Reference

| Command | Subcommand / Options | Description |
| :--- | :--- | :--- |
| `dbs distro` | `init`, `build`, `publish`, `serve`, `status` | End-to-end distribution repository lifecycle: initialization, DAG build, publication, GPG signing, and HTTP/Nginx serving. |
| `dbs explore` | `--distro`, `--search`, `--limit` | Search remote packages across Pagure, GitLab, or Forgejo APIs. |
| `dbs distgit clone` | `--distro`, `--as`, `--rename-spec`, `--new-origin` | Clone dist-git repositories with optional renaming and remote setup. |
| `dbs distgit sync` | `-j`, `--sources`, `--search`, `--record-db` | Batch synchronize multiple repositories and lookaside sources concurrently. |
| `dbs distgit pull` | `-o <dest>` | Pull git updates for all cloned repositories in the destination folder. |
| `dbs distgit inspect` | `<path>` | Parse `.spec` file and display EVR, sources, patches, and dependencies. |
| `dbs dag` | `-i <path>`, `--report`, `--build`, `--fetch-sources` | Compute topological build order (DAG), auto-cache sources in lookaside, and execute layered builds. |
| `dbs build` | `-r <chroot>`, `-j <workers>`, `--chain`, `--fetch-sources` | Compile packages in Mock or rpmbuild with auto-lookaside source caching and dynamic repo feedback. |
| `dbs lookaside` | `upload`, `get`, `sync`, `status`, `gc` | Maintain Content-Addressable Storage (CAS) for source archives with BTRFS CoW reflinks. |
| `dbs chroot` / `mock` | `list`, `inspect`, `check`, `add`, `init` | Discover, inspect, validate, and manage custom Mock chroot configurations. |
| `dbs db` | `bootstrap`, `status`, `reset`, `dump-schema` | Bootstrap embedded database schema, inspect table health, reset, or export raw SQL. |
| `dbs os` | `list`, `add`, `delete` | Manage operating system distribution definitions and presets. |
| `dbs pkg` | `list`, `add`, `delete` | Query and manage packages in the PostgreSQL supply chain catalog. |

---

## Hands-On Tutorial

For a complete step-by-step walkthrough detailing how to create, re-remote, resolve dependencies, and compile packages for a distribution, see [TUTORIAL.md](TUTORIAL.md).

---

## Development

```bash
make dirs      # Prepare data/ and bin/ directories
make test      # Run all unit tests
make build     # Compile debug binary
make release   # Compile optimized release binary into ./bin/dbs
make clean     # Clean target and generated artifacts
```

### Database Provisioning & Management (Optional)

DBS can track all packages, capabilities, build durations, and artifacts in PostgreSQL.
When distributing `dbs`, **no Makefile or loose SQL scripts are required**—the complete schema is embedded directly into the binary:

```bash
# 1. Bootstrap schema and default seeds (reads DATABASE_URL, /etc/dbs/dbs.env, or .env):
./bin/dbs db bootstrap

# 2. Check database connectivity, PostgreSQL version, and table counts:
./bin/dbs db status

# 3. Automated host or container provisioning:
sudo ./scripts/setup_db.sh --mode host       # For native PostgreSQL on host
./scripts/setup_db.sh --mode container      # For Podman container
```

---

## License

Licensed under the Apache License, Version 2.0 or GNU General Public License v3.0+.
