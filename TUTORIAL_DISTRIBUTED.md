# DBS (Distribution Build System) - Distributed Cluster Compilation Tutorial

Welcome to the hands-on tutorial for **Distributed Compilation with the Distribution Build System (DBS)**. This guide provides an end-to-end walkthrough for deploying, orchestrating, and scaling package builds across a compilation cluster using an orchestrator node (e.g. `sohoai`) and any number of worker nodes running `dbs worker`.

---

## Table of Contents

1. [Architecture & Design Principles](#1-architecture--design-principles)
2. [Cluster Prerequisites & Networking](#2-cluster-prerequisites--networking)
3. [Quickstart: 2-Node Cluster Walkthrough](#3-quickstart-2-node-cluster-walkthrough)
4. [Orchestrator Setup & Execution (`dbs distro build --distributed`)](#4-orchestrator-setup--execution-dbs-distro-build---distributed)
5. [Worker Node Setup & Execution (`dbs worker`)](#5-worker-node-setup--execution-dbs-worker)
6. [Topological Layering & Dynamic Remote Repositories](#6-topological-layering--dynamic-remote-repositories)
7. [Real-Time Monitoring & Telemetry (`dbs monitor` / `dbs top`)](#7-real-time-monitoring--telemetry-dbs-monitor--dbs-top)
8. [Failure Recovery, Heartbeats & Stale Build Reclaiming](#8-failure-recovery-heartbeats--stale-build-reclaiming)
9. [Running Workers as Systemd Daemons](#9-running-workers-as-systemd-daemons)
10. [Hardware Tuning & Production Best Practices](#10-hardware-tuning--production-best-practices)
11. [Troubleshooting](#11-troubleshooting)

---

## 1. Architecture & Design Principles

DBS distributed compilation implements a **pull-based, lockless central queue** backed by PostgreSQL and an embedded streaming HTTP coordinator service. 

Traditional distributed build orchestrators often require complex SSH orchestration, push-based scheduling, or remote lookaside cache replicas across every worker. DBS simplifies this dramatically:

```mermaid
flowchart TD
    subgraph Orchestrator ["Orchestrator Node (sohoai)"]
        DAG[Kahn's DAG Engine<br/>Topological Layering]
        SRPM[Hermetic SRPM Generator<br/>staging/srpms/]
        HTTP[Embedded HTTP Coordinator<br/>Port 8080]
        DB[(PostgreSQL Catalog & Queue<br/>FOR UPDATE SKIP LOCKED)]
        LocalW[Optional Local Workers<br/>-j 4]
    end

    subgraph Cluster ["Worker Nodes (Cluster / LAN)"]
        W1["Worker Node 1<br/>dbs worker -j 8"]
        W2["Worker Node 2<br/>dbs worker -j 16"]
        W3["Worker Node N<br/>dbs worker -j 4"]
    end

    DAG -->|1. Compute Layers| SRPM
    SRPM -->|2. Enqueue PENDING| DB
    SRPM -->|3. Expose .src.rpm| HTTP

    DB <-->|Atomic Claim & Heartbeats| W1
    DB <-->|Atomic Claim & Heartbeats| W2
    DB <-->|Atomic Claim & Heartbeats| W3
    DB <-->|Local Claim| LocalW

    HTTP -->|Fetch .src.rpm| W1
    HTTP -->|Fetch .src.rpm| W2
    HTTP -->|Fetch .src.rpm| W3

    HTTP -.->|Dynamic --addrepo feedback| W1
    HTTP -.->|Dynamic --addrepo feedback| W2
    HTTP -.->|Dynamic --addrepo feedback| W3

    W1 -->|PUT /upload RPMs & Logs| HTTP
    W2 -->|PUT /upload RPMs & Logs| HTTP
    W3 -->|PUT /upload RPMs & Logs| HTTP
```

### Architectural Highlights

1. **Self-Contained SRPMs:** Workers do not need access to upstream dist-git Git repos or multi-gigabyte lookaside caches. The orchestrator packages each spec, patches, and cached tarballs into a hermetic `.src.rpm` inside `<staging_dir>/srpms/`. Workers download only the `.src.rpm` and compile it with `mock --rebuild`.
2. **Lockless Atomic Queue (`SKIP LOCKED`):** Workers claim jobs using PostgreSQL's `SELECT ... FOR UPDATE SKIP LOCKED LIMIT 1`. This completely eliminates worker lock contention and race conditions without needing a separate message broker (e.g. RabbitMQ or Redis).
3. **Dynamic Layer Feedback (`--addrepo`):** As Layer $K$ packages finish compiling, the coordinator indexes them (`createrepo_c`) and exposes them over HTTP. Layer $K+1$ workers receive `--addrepo=http://coordinator:8080/staging/rpms/x86_64`, allowing `mock` to resolve freshly built build-time dependencies automatically.
4. **Streaming Uploads:** When Mock finishes, workers stream the resulting RPMs and build logs directly to the coordinator via HTTP `PUT /upload/staging/rpms/...`, bypassing memory buffers.
5. **Worker Heartbeats & Stale Build Recovery:** Workers register in the `cluster_worker` table and send periodic heartbeats. If a worker crashes or drops off the network, the orchestrator automatically re-enqueues its in-flight packages back to `PENDING`.

---

## 2. Cluster Prerequisites & Networking

### Network Requirements
* **PostgreSQL (Port 5432):** The orchestrator's PostgreSQL instance must be accessible by worker nodes over your local network.
* **HTTP Coordinator (Port 8080 default):** Workers must be able to reach the coordinator's HTTP port to download SRPMs, query the dynamic package repository, and upload artifacts.

> [!TIP]
> Ensure host firewalls (e.g., `firewalld` or `nftables`) permit traffic on ports `5432` and `8080`:
> ```bash
> sudo firewall-cmd --add-port=5432/tcp --permanent
> sudo firewall-cmd --add-port=8080/tcp --permanent
> sudo firewall-cmd --reload
> ```

### PostgreSQL Configuration (`pg_hba.conf`)
To allow remote workers to authenticate, add a line to your PostgreSQL `pg_hba.conf` on the orchestrator:
```text
# Allow dbs workers from the local subnet
host    dbs             dbs             192.168.1.0/24          scram-sha-256
```
Then reload PostgreSQL:
```bash
sudo systemctl reload postgresql
```

### Worker Host Requirements
Each compilation node only needs:
1. **Mock and Build Tools:**
   ```bash
   sudo dnf install -y mock
   sudo usermod -a -G mock $USER
   newgrp mock
   ```
2. **The `dbs` Binary:**
   Copy `./bin/dbs` to `/usr/local/bin/dbs` or run it from the local workspace.
3. **Target Mock Configuration:**
   Copy the target chroot configuration (e.g. `mock/tacos-stable-x86_64.cfg`) or ensure system configurations exist in `/etc/mock/`.

---

## 3. Quickstart: 2-Node Cluster Walkthrough

In this scenario:
* **Node 1 (`sohoai` / `192.168.1.50`):** Acts as Orchestrator, PostgreSQL host, and coordinator HTTP server.
* **Node 2 (`worker-01` / `192.168.1.51`):** Acts as a dedicated compilation node with 16 cores.

### Step 1: Start the Orchestrator on `sohoai`
Run `dbs distro build` with the `--distributed` flag:

```bash
./bin/dbs distro build tacos-stable-x86_64 \
  --config tacos-distro.toml \
  --distributed \
  --coordinator-port 8080 \
  --coordinator-url "http://192.168.1.50:8080" \
  --local-workers 2 \
  --record-db
```

The orchestrator:
1. Resolves the DAG dependency layers.
2. Spawns the embedded HTTP coordinator at `http://0.0.0.0:8080`.
3. Prepares Layer 0 SRPMs and enqueues them as `PENDING` in PostgreSQL.
4. Begins monitoring layer progress and executing 2 local worker slots.

### Step 2: Start the Worker Node on `worker-01`
On `worker-01`, start the worker pointing to `sohoai`:

```bash
dbs worker \
  --db "postgres://dbs:mG3A5X034CYPmD4anMsH03P7@192.168.1.50:5432/dbs" \
  --coordinator-url "http://192.168.1.50:8080" \
  --root tacos-stable-x86_64 \
  --node-name "worker-01" \
  --worker-id 101 \
  -j 4 \
  --smp 8
```

The worker:
1. Registers itself in `cluster_worker` and starts sending heartbeats every 10 seconds.
2. Atomically claims available `PENDING` packages.
3. Downloads `.src.rpm` from `http://192.168.1.50:8080/staging/srpms/`.
4. Compiles them hermetically inside Mock chroots.
5. Uploads generated `.rpm` packages and `.log` files back to `sohoai`.

---

## 4. Orchestrator Setup & Execution (`dbs distro build --distributed`)

The orchestrator is invoked through `dbs distro build` using distributed flags:

```bash
dbs distro build [NAME] [TARGETS]... [OPTIONS]
```

### Orchestrator Command Flags

| Flag | Default | Description |
| :--- | :--- | :--- |
| `--distributed` | `false` | Enables cluster orchestration, embedded HTTP coordinator, and PostgreSQL queue dispatching. |
| `--coordinator-port <PORT>` | `8080` | Port for the embedded HTTP coordinator service. |
| `--coordinator-url <URL>` | `http://127.0.0.1:<port>` | The public or LAN URL that workers use to contact the coordinator (e.g. `http://192.168.1.50:8080`). |
| `--local-workers <N>` | `distro.workers` | Number of concurrent local worker slots running on the orchestrator host itself. Set to `0` to keep the orchestrator strictly dedicated to coordination. |
| `-j, --concurrency <N>` | `distro.workers` | Concurrency limit for local DAG operations and build workers. |
| `--record-db` | `false` | Persists build state, packages, dependencies, and worker metrics to PostgreSQL. **Required for distributed builds**. |
| `--stage <STAGE>` | `None` | Restricts distributed compilation to a specific distribution stage (e.g. `bootstrap`, `system`, `desktop`). |
| `-p, --packages <FILE>` | `None` | Builds from an explicit newline-delimited package list file instead of the full tree. |
| `--nocheck` | `false` | Instructs all Mock builds to bypass `%check` test suites for maximum build throughput. |

### Dedicated Orchestrator Mode (Zero Local Workers)
On lightweight orchestrators or when reserving the coordinator solely for lookaside, database, and repository indexing:

```bash
dbs distro build tacos-stable-x86_64 \
  --config tacos-distro.toml \
  --distributed \
  --local-workers 0 \
  --coordinator-port 8080 \
  --coordinator-url "http://192.168.1.50:8080" \
  --record-db
```

### Target Selection (Comps Groups and Single Packages)
You can target specific packages or comps groups in distributed mode:

```bash
# Build the @core environment across the cluster
dbs distro build tacos-stable-x86_64 @core --distributed --coordinator-port 8080 --record-db

# Build a list of packages
dbs distro build tacos-stable-x86_64 -p build_order.txt --distributed --coordinator-port 8080 --record-db
```

---

## 5. Worker Node Setup & Execution (`dbs worker`)

The `dbs worker` command connects to PostgreSQL, claims packages, compiles them in hermetic Mock chroots, and uploads artifacts to the coordinator.

```bash
dbs worker [OPTIONS]
```

### Worker Command Flags

| Flag | Environment Variable | Default | Description |
| :--- | :--- | :--- | :--- |
| `--db <URL>` | `DATABASE_URL` | Config file URL | PostgreSQL connection string (e.g. `postgres://dbs:pass@sohoai:5432/dbs`). |
| `--coordinator-url <URL>` | `DBS_COORDINATOR_URL` | `http://127.0.0.1:8080` | Base URL of the coordinator HTTP server. |
| `-r, --root <ROOT>` | — | `[chroot].profile` | Name of the Mock chroot profile (e.g. `tacos-stable-x86_64`). |
| `--configdir <DIR>` | — | `mock` / `/etc/mock` | Custom directory containing Mock `.cfg` files. |
| `-j, --concurrency <N>` | — | `4` | Number of concurrent package compilation slots on this worker. |
| `--smp <N>` | `DBS_SMP_CPUS` | System CPUs | Number of SMP compilation cores passed to Mock inside each chroot (`%_smp_mflags`). |
| `--worker-id <ID>` | — | `100 + (PID % 800)` | Unique numeric identifier for telemetry attribution. |
| `--node-name <NAME>` | `HOSTNAME` | System Hostname | Human-readable node name displayed in telemetry and logs. |
| `--heartbeat-interval <S>` | — | `10` | Heartbeat frequency in seconds reported to PostgreSQL. |
| `--poll-interval <S>` | — | `3` | Seconds to wait between polling attempts when the build queue is empty. |
| `--work-dir <DIR>` | — | `/tmp/dbs-worker-<id>` | Local scratch directory for downloading SRPMs and building artifacts. |
| `--keep-build-dir` | — | `false` | Retains temporary build directories after uploading instead of cleaning up. |
| `--nocheck` | — | `false` | Bypasses `%check` test suites during compilation on this worker. |
| `--nocheck-pkg <NAMES>` | — | `[]` | Comma-separated or repeated package names exempt from test suites. |

### Sizing Concurrency and SMP
For optimal throughput on a multi-core worker node:
* **High-RAM, High-Core Node (e.g. 32 cores, 64 GB RAM):**
  Run `-j 4` concurrent slots with `--smp 8`, or `-j 8` with `--smp 4`.
  ```bash
  dbs worker --coordinator-url http://sohoai:8080 -j 8 --smp 4 --worker-id 201
  ```
* **Memory-Constrained Node (e.g. 8 cores, 16 GB RAM):**
  Run `-j 2` concurrent slots with `--smp 4` to prevent OOM when compiling heavy C++/Rust packages.
  ```bash
  dbs worker --coordinator-url http://sohoai:8080 -j 2 --smp 4 --worker-id 202
  ```

---

## 6. Topological Layering & Dynamic Remote Repositories

DBS schedules builds according to **Kahn's in-degree topological DAG**:

```
Layer 0:  [glibc, kernel-headers, filesystem, setup]    <-- Independent packages
                              │
                    (Compiled & Indexed)
                              ▼
Layer 1:  [binutils, bash, coreutils, zlib]             <-- Depend on Layer 0
                              │
                    (Compiled & Indexed)
                              ▼
Layer 2:  [gcc, make, tar, findutils]                   <-- Depend on Layer 1
```

### How Layer Feedback Works Across the Network

1. **Layer Generation:** The orchestrator identifies all packages in **Layer $K$** with in-degree 0. It generates their `.src.rpm` files and flags them as `PENDING` in the database.
2. **Cluster Execution:** Workers pull and build Layer $K$ packages concurrently.
3. **Barrier & Dynamic Indexing:** The orchestrator pauses at the layer boundary until all packages in Layer $K$ transition to `SUCCESS` or `FAILED`.
4. **Repository Regeneration:** Once Layer $K$ completes, the orchestrator indexes `<staging_dir>/rpms/x86_64` using `createrepo_c`.
5. **Next Layer Injection:** When Layer $K+1$ packages are claimed, `MockRunner` automatically receives:
   ```bash
   --addrepo=http://<coordinator-url>/staging/rpms/x86_64
   ```
   Mock dynamically queries this repository during `dnf builddep`, resolving newly built libraries without manual intervention!

---

## 7. Real-Time Monitoring & Telemetry (`dbs monitor` / `dbs top`)

DBS includes a full-screen interactive terminal dashboard for live cluster telemetry:

```bash
dbs monitor
# or
dbs top
```

### Dashboard Views

* **Overview (Tab 1):** Real-time summary of total packages, build status breakdown (`PENDING`, `BUILDING`, `SUCCESS`, `FAILED`, `SKIPPED`), active workers, and cluster throughput.
* **Builds Pipeline (Tab 2):** Live list of in-flight compilations showing package name, assigned worker (`W101`, `W102`), duration, and log path.
* **Cluster Workers (Tab 3):** List of registered cluster nodes showing node hostname, worker ID, concurrent slots, last heartbeat timestamp, and health status (`ACTIVE`, `STALE`, `OFFLINE`).

### Command-Line Status Check
To view repository status and package counts without launching the interactive TUI:

```bash
dbs distro status tacos-stable-x86_64
```

---

## 8. Failure Recovery, Heartbeats & Stale Build Reclaiming

A major challenge in distributed build systems is worker failure (power loss, network partition, kernel panic, or OOM kill).

### Heartbeat Lifecycle
1. When a worker starts, it registers in `cluster_worker`:
   ```sql
   INSERT INTO cluster_worker (worker_id, node_name, status, last_heartbeat)
   VALUES (101, 'epyc-node-01', 'ACTIVE', NOW());
   ```
2. Every `--heartbeat-interval` seconds (default 10s), the worker runs:
   ```sql
   UPDATE cluster_worker SET last_heartbeat = NOW() WHERE worker_id = 101;
   ```
3. When shutting down cleanly (`Ctrl+C` or `SIGTERM`), the worker updates its status to `OFFLINE`.

### Stale Build Reclaiming
If a node drops off the network without cleanly unregistering:
1. The orchestrator's monitoring loop scans for workers whose `last_heartbeat` is older than **300 seconds** (5 minutes).
2. It executes `db::reclaim_stale_builds`:
   ```sql
   UPDATE package
   SET build_status = 'PENDING', worker_id = NULL
   WHERE build_status = 'BUILDING'
     AND worker_id IN (
         SELECT worker_id FROM cluster_worker
         WHERE last_heartbeat < NOW() - INTERVAL '300 seconds'
     );
   ```
3. Healthy workers immediately pick up the reclaimed packages and resume compilation without stalling the entire pipeline.

---

## 9. Running Workers as Systemd Daemons

To run `dbs worker` continuously on dedicated build servers, create a systemd service unit:

### `/etc/systemd/system/dbs-worker.service`
```ini
[Unit]
Description=DBS Distributed Compilation Worker
After=network.target network-online.target
Wants=network-online.target

[Service]
Type=simple
User=builder
Group=mock
WorkingDirectory=/home/builder
Environment="DATABASE_URL=postgres://dbs:mG3A5X034CYPmD4anMsH03P7@192.168.1.50:5432/dbs"
Environment="DBS_COORDINATOR_URL=http://192.168.1.50:8080"
ExecStart=/usr/local/bin/dbs worker \
    --root tacos-stable-x86_64 \
    --concurrency 4 \
    --smp 8 \
    --node-name %H \
    --work-dir /var/tmp/dbs-worker
Restart=always
RestartSec=5s
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

### Enable and Start the Service
```bash
sudo systemctl daemon-reload
sudo systemctl enable --now dbs-worker
sudo systemctl status dbs-worker
```

Inspect worker logs:
```bash
journalctl -u dbs-worker -f
```

---

## 10. Hardware Tuning & Production Best Practices

### 1. Fast Chroot Storage with BTRFS CoW
Place Mock's working directory (`/var/lib/mock`) on a BTRFS filesystem. Mock uses BTRFS snapshot subvolumes for instantaneous chroot initialization, dropping per-package setup time from 30 seconds to under 1 second.

### 2. Tmpfs Mount for Worker Results
For workers with ample RAM, mount the temporary work directory on `tmpfs`:
```bash
sudo mount -t tmpfs -o size=16G tmpfs /var/tmp/dbs-worker
```

### 3. PostgreSQL Connection Scaling
Each concurrent worker slot opens short-lived connections to PostgreSQL. If running 10 nodes with 8 slots each (80 concurrent workers), increase PostgreSQL's `max_connections` on the orchestrator:

Edit `/var/lib/pgsql/data/postgresql.conf`:
```ini
max_connections = 250
shared_buffers = 4GB
work_mem = 64MB
```
Restart PostgreSQL:
```bash
sudo systemctl restart postgresql
```

### 4. Bypassing Checks on Heavy Leaves (`--nocheck`)
Certain packages run exhaustive test suites that require hours to complete (e.g. `gcc`, `valgrind`, `rust`). You can exempt them on the command line or in your config file:
```bash
dbs worker --coordinator-url http://sohoai:8080 -j 4 --nocheck-pkg gcc,rust,llvm
```

---

## 11. Troubleshooting

### Worker Cannot Connect to PostgreSQL
* **Error:** `connection to server at "192.168.1.50", port 5432 failed: Connection refused`
* **Checks:**
  1. Verify PostgreSQL is listening on `0.0.0.0` or the LAN IP in `postgresql.conf`:
     ```ini
     listen_addresses = '*'
     ```
  2. Verify `pg_hba.conf` contains an entry allowing the worker's subnet.
  3. Verify port 5432 is open in the orchestrator's firewall.

### Worker Fails to Download SRPM
* **Error:** `Failed to download SRPM from coordinator at http://192.168.1.50:8080/staging/srpms/...`
* **Checks:**
  1. Ensure the orchestrator was launched with `--coordinator-url "http://192.168.1.50:8080"` and not `127.0.0.1`.
  2. Test connectivity from the worker:
     ```bash
     curl -I http://192.168.1.50:8080/health
     ```

### Mock Group Permission Denied
* **Error:** `mock: error: Cannot find user in mock group`
* **Solution:**
  Run `sudo usermod -a -G mock $USER` on the worker node, then log out and log back in (or run `newgrp mock`).

### Package Fails Build-Dependency Resolution (`dnf builddep`)
* **Cause:** The package requires a capability built in an earlier layer that has not yet reached the staging repository.
* **Solution:**
  Verify the orchestrator completed indexing for the previous layer. Check the coordinator's staging repository metadata:
  ```bash
  curl -s http://192.168.1.50:8080/staging/rpms/x86_64/repodata/repomd.xml
  ```

---

## Next Steps

* Read the single-node guide in [TUTORIAL.md](TUTORIAL.md).
* Review internal agent guidelines and CLI references in [AGENTS.md](AGENTS.md).
* Examine common build diagnostics and error extraction in [TROUBLESHOOTING.md](TROUBLESHOOTING.md).
