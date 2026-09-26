# Distribution Build System (DBS) - Agent Specifications & Context

## 1. Project Overview
Welcome to the **Distribution Build System (DBS)** workspace. This document is the primary architectural guide and system context for **Antigravity CLI** (`agy`) agents operating within this repository.

**DBS** is a high-throughput, distribution-agnostic Linux distribution build system written in Rust. It eliminates opaque, ad-hoc distribution bootstrapping scripts ("secret sauces") by providing a unified, declarative pipeline for discovering, synchronizing, inspecting, and compiling packages directly from **dist-git** repositories across any RPM-based distribution (Fedora Rawhide, CentOS Stream, TacOS, RHEL, etc.).

---

## 2. Architecture & Tech Stack

* **Language & Toolchain:** Rust (edition 2024), Cargo, Tokio async runtime.
* **Database & ORM:** PostgreSQL with [Diesel](https://diesel.rs/) ORM (`diesel`).
* **CLI Parser:** [Clap](https://crates.io/crates/clap) (v4 with derive macros).
* **Network & Lookaside:** [Reqwest](https://crates.io/crates/reqwest) for Pagure/GitLab APIs and lookaside cache source archive downloads.
* **Build Runners (`BuildRunner` trait):**
  * `MockRunner`: Primary enterprise-grade hermetic chroot builds with `--chain` and dynamic local repository feedback (`--addrepo`).
  * `RpmbuildRunner`: Direct host-level execution for rapid debugging.
* **Spec & RPM Engine:** Pure-Rust `.spec` file parser (`distgit::spec`) for dependency resolution and capability extraction without host tool dependencies.
* **DAG & Build Ordering Engine:** Kahn's in-degree topological sort algorithm (`dag::DependencyGraph`) for computing parallel compilation layers with deterministic scheduling and explicit cycle detection.

---

## 3. Database Rules & Schema Synchronization

### Invariant Rules
1. **Migration Parity:** The SQL definitions in `sql/` and `rust/migrations/2025-07-03-050157_supply_chain/` **MUST ALWAYS REMAIN 100% IDENTICAL**:
   * `sql/schema_up.sql` == `rust/migrations/2025-07-03-050157_supply_chain/up.sql`
   * `sql/schema_down.sql` == `rust/migrations/2025-07-03-050157_supply_chain/down.sql`
2. **RPM Database Schema Compatibility:**
   * Package state tracking uses the `build_status` enum: `PENDING`, `BUILDING`, `SUCCESS`, `FAILED`, `SKIPPED`.
   * Package metadata captures RPM EVR (`epoch`, `version`, `release`), source RPM names (`sourcerpm`), dist-git commit hashes, and spec paths.
   * Granular capabilities are tracked in dedicated relational tables:
     * `package_provides`: Exported capabilities (`name`, `flags`, `version`).
     * `package_requires`: Required dependencies (`name`, `flags`, `version`, `is_build_require`).
     * `package_artifact`: Staged/published binary and source RPM artifacts (`rpm_filename`, `rpm_path`, `arch`, `file_size_bytes`).

---

## 4. Dist-Git Integration Matrix

DBS abstracts dist-git backends using `DistroConfig` and `DistGitClient`:

| Distribution | Default Branch | Dist-Git Clone Template | API Provider | Lookaside Cache Base |
| :--- | :--- | :--- | :--- | :--- |
| **Fedora Rawhide** | `rawhide` | `https://src.fedoraproject.org/rpms/{package}.git` | Pagure (`/api/0/projects`) | `https://src.fedoraproject.org/repo/pkgs` |
| **CentOS Stream 10** | `c10s` | `https://gitlab.com/redhat/centos-stream/rpms/{package}.git` | GitLab (Group `8794173`) | `https://sources.stream.centos.org/sources/rpms` |
| **CentOS Stream 9** | `c9s` | `https://gitlab.com/redhat/centos-stream/rpms/{package}.git` | GitLab (Group `8794173`) | `https://sources.stream.centos.org/sources/rpms` |
| **TacOS Rolling** | `master` | `https://codeberg.org/imcsk8/tacos.git` | Forgejo (`/api/v1`) | `https://repos.tacos.org.mx/sources` |

---

## 5. Build Runner Evaluation & Trade-Offs

When selecting build runners for distribution packages:

1. **Mock (`MockRunner`) - RECOMMENDED for Production Builds:**
   * **Pros:** Hermetic chroot isolation (`systemd-nspawn`), prevents undeclared build requirements, cross-distribution support (e.g. build CentOS on Fedora host), native `--chain` support and dynamic local repository feedback (`--addrepo=file://...`).
   * **Cons:** Requires `mock` group permissions and chroot creation overhead.
2. **Host `rpmbuild` (`RpmbuildRunner`):**
   * **Pros:** Fast, zero chroot setup overhead.
   * **Cons:** Pollutes host system, cannot easily build packages targeting a different distribution or glibc than the host, unsafe for untrusted specs.
3. **Pure Rust / `librpm.rs` / `rpm-rs`:**
   * **Pros:** Ideal for inspecting `.spec` files, parsing RPM headers, extracting EVR/Requires/Provides, and packing files into binary RPMs without external tools.
   * **Cons:** Cannot compile arbitrary C/C++/Rust source trees on its own without invoking compilers and toolchains.

---

## 6. CLI Command Reference

Execute commands using `cargo run --` or `./rust/bin/dbs`:

### Remote Package Explorer
```bash
# Search packages across Fedora Rawhide
dbs explore --distro fedora-rawhide --search kernel --limit 10

# Search packages across CentOS Stream 10
dbs explore --distro centos-stream-10 --search zstd --limit 10
```

### Dist-Git Repository Management
```bash
# Clone a package repository
dbs distgit clone --distro centos-stream-10 -o data/distgit zstd

# Clone upstream package under a new name (e.g. create a TacOS dist-git repo from Fedora Rawhide)
dbs distgit clone --distro fedora-rawhide zstd --as tacos-zstd --rename-spec

# Clone and configure new origin remote (e.g. Codeberg/Forgejo) while preserving upstream
dbs distgit clone --distro fedora-rawhide kernel --as tacos-kernel --new-origin https://codeberg.org/imcsk8/tacos-kernel.git

# Clone multiple packages with source:target mapping syntax
dbs distgit clone --distro fedora-rawhide fedora-release:tacos-release zstd:tacos-zstd

# Synchronize multiple repositories in parallel with worker pool & lookaside cache
dbs distgit sync --distro fedora-rawhide -j 8 --sources --search python --limit 20

# Pull updates for all cloned repositories
dbs distgit pull -o data/distgit

# Inspect spec file metadata and dependencies
dbs distgit inspect data/distgit/zstd/zstd.spec
```

### Package Building
### Package Building & Orchestration
```bash
# Build a single package in Mock chroot
dbs build -r fedora-rawhide-x86_64 -o staging package.src.rpm

# Sequential Mock chain build with dynamic dependency resolution
dbs build -r fedora-rawhide-x86_64 --chain -c -o staging pkg1.src.rpm pkg2.src.rpm

# Parallel Mock worker pool with dynamic local repository feedback
dbs build -r centos-stream-10-x86_64 -j 4 --dynamic-repo -o staging pkg1.spec pkg2.spec

# Record build metrics and RPM artifacts to PostgreSQL database
dbs build -r fedora-rawhide-x86_64 --record-db -o staging pkg.spec
```

### Dependency Graph (DAG) & Layered Build Orchestration
```bash
# Analyze dependencies across .spec files and display parallel compilation layers
dbs dag -i data/distgit

# Generate markdown report detailing dependencies, cycles, and layers
dbs dag -i data/distgit --report reports/dependency_graph.md

# Compute compilation layers and immediately build packages layer-by-layer
dbs dag -i data/distgit --build -r fedora-rawhide-x86_64 -j 4 -o staging --record-db
```

### Mock Chroot Configuration Management
```bash
# List all discovered Mock chroots across workspace, project, and user search paths
dbs chroot list

# Include system defaults from /etc/mock
dbs chroot list --all

# Inspect a chroot configuration profile or .cfg file
dbs chroot inspect tacos-rolling-x86_64

# Check and validate a chroot configuration with Mock
dbs chroot check tacos-rolling-x86_64
dbs chroot check mock/tacos-rolling-x86_64.cfg

# Import custom chroots into workspace (./mock)
dbs chroot add ../tacos/mock/tacos-rolling-x86_64.cfg

# Scaffold a new distribution chroot template
dbs chroot init my-distro-x86_64 --arch x86_64
```

### Dist-Git Lookaside Cache Management (BTRFS CoW)
```bash
# Display lookaside metrics, CAS storage, and BTRFS filesystem status
dbs lookaside status

# Upload source archive, compute SHA-512, store in CAS, and update dist-git 'sources'
dbs lookaside upload --pkg zstd --file /path/to/zstd-1.5.7.tar.gz --spec data/distgit/zstd/zstd.spec

# Concurrently pre-fetch missing source archives across cloned repositories
dbs lookaside sync -i data/distgit -j 4

# Retrieve archive by SHA-512 into local path
dbs lookaside get --pkg zstd --file zstd-1.5.7.tar.gz --hash <sha512> --dest ./SOURCES/

# Prune unreferenced/orphaned source archives from CAS
dbs lookaside gc --dry-run --distgit data/distgit
```

### Distribution Catalog & Package Management
```bash
# List supported distribution presets and registered database records
dbs distro list

# Register a new distribution in the database catalog
dbs distro add --name "TacOS" --version "1.0" --release "rolling" --architecture "x86_64" --distro-tag "tcrs"

# Query package catalog in the database
dbs pkg list
```

---

## 7. Developer Guidelines
* **Skill Compliance:** Always follow the `rust-developer` skill rules:
  * Thorough `rustdoc` (`///` or `//!`) comments on all public structs and functions.
  * Explicit `match` error propagation for semantic clarity (avoid unhandled `.map_err(...)?`).
  * Simplicity and modular organization.
* **Makefile Targets:**
  * `make dirs`: Creates required workspace directories (`data`, `bin`).
  * `make db`: Starts development PostgreSQL container.
  * `make bootstrap`: Runs database migrations up.
  * `make clean_db`: Runs database migrations down.
  * `make build`: Compiles debug binary.
  * `make test`: Runs all unit tests.
  * `make release`: Compiles optimized release binary into `bin/dbs` and `rust/bin/dbs`.
  * `make clean`: Cleans generated files.
