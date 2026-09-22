#!/usr/bin/env python3

import os
import sys
import re
import urllib.request
import urllib.parse
import lzma
import sqlite3
import argparse
import tempfile
import xml.etree.ElementTree as ET

# Increase recursion depth for Tarjan's algorithm on large graphs
sys.setrecursionlimit(10000)

def parse_srpm_name(srpm):
    if not srpm:
        return None
    # Standard format: name-version-release.src.rpm
    m = re.match(r'^(.*?)-[^-]+-[^-]+\.src\.rpm$', srpm)
    if m:
        return m.group(1)
    parts = srpm.split('-')
    if len(parts) > 2:
        return '-'.join(parts[:-2])
    return srpm

def get_sqlite_url(repo_url):
    """
    Downloads repomd.xml from the repository URL and extracts the primary sqlite file path.
    """
    repomd_url = urllib.parse.urljoin(repo_url, 'repodata/repomd.xml')
    headers = {'User-Agent': 'Mozilla/5.0'}
    req = urllib.request.Request(repomd_url, headers=headers)

    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            xml_content = r.read()
    except Exception as e:
        print(f"Error fetching repomd.xml from {repomd_url}: {e}", file=sys.stderr)
        return None

    root = ET.fromstring(xml_content)
    # Define namespace map for repomd.xml
    ns = {'repo': 'http://linux.duke.edu/metadata/repo'}

    # Look for primary database element
    for data in root.findall('repo:data', ns):
        if data.get('type') == 'primary_db':
            location = data.find('repo:location', ns)
            if location is not None:
                href = location.get('href')
                return urllib.parse.urljoin(repo_url, href)

    # Fallback to checking location with primary type if primary_db is missing
    for data in root.findall('repo:data', ns):
        if data.get('type') == 'primary':
            location = data.find('repo:location', ns)
            if location is not None:
                href = location.get('href')
                if href.endswith('.sqlite.xz') or href.endswith('.sqlite.gz'):
                    return urllib.parse.urljoin(repo_url, href)

    print(f"Could not find primary database URL in {repomd_url}", file=sys.stderr)
    return None

def download_and_decompress(url, dest_db_path):
    """
    Downloads the xz compressed sqlite file and decompresses it to dest_db_path.
    """
    headers = {'User-Agent': 'Mozilla/5.0'}
    req = urllib.request.Request(url, headers=headers)

    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            compressed_data = r.read()
    except Exception as e:
        print(f"Failed to download metadata from {url}: {e}", file=sys.stderr)
        return False

    print(f"Decompressing metadata database...")
    try:
        decompressed = lzma.decompress(compressed_data)
        with open(dest_db_path, 'wb') as f:
            f.write(decompressed)
        return True
    except Exception as e:
        print(f"Decompression failed: {e}", file=sys.stderr)
        return False

def analyze_dependencies(tmp_dir, version, arch):
    # CentOS Stream mirror URLs for BaseOS and AppStream
    repo_urls = {
        #'src_baseos': f'https://mirror.stream.centos.org/{version}-stream/BaseOS/source/tree/',
        #'src_appstream': f'https://mirror.stream.centos.org/{version}-stream/AppStream/source/tree/',
        'bin_baseos': f'https://mirror.stream.centos.org/{version}-stream/BaseOS/{arch}/os/',
        'bin_appstream': f'https://mirror.stream.centos.org/{version}-stream/AppStream/{arch}/os/'
    }

    dbs = {}
    for name, base_url in repo_urls.items():
        print(f"Resolving metadata for {name}...")
        sqlite_url = get_sqlite_url(base_url)
        if not sqlite_url:
            print(f"Failed to resolve SQLite metadata URL for {name}", file=sys.stderr)
            return None, None

        db_path = os.path.join(tmp_dir, f"{name}.sqlite")
        print(f"Downloading {name} database...")
        if not download_and_decompress(sqlite_url, db_path):
            return None, None
        dbs[name] = db_path

    # 1. Load binary packages and provides map
    provides_map = {}
    binary_pkg_to_srpm = {}

    print("Parsing binary package metadata and capabilities...")
    for key in ['bin_baseos', 'bin_appstream']:
        conn = sqlite3.connect(dbs[key])
        cursor = conn.cursor()

        cursor.execute('SELECT pkgKey, name, rpm_sourcerpm FROM packages;')
        pkg_srpm = {}
        for pkgKey, name, rpm_sourcerpm in cursor.fetchall():
            srpm_name = parse_srpm_name(rpm_sourcerpm)
            if srpm_name:
                pkg_srpm[pkgKey] = srpm_name
                binary_pkg_to_srpm[name] = srpm_name
                provides_map.setdefault(name, set()).add(srpm_name)

        cursor.execute('SELECT name, pkgKey FROM provides;')
        for prov_name, pkgKey in cursor.fetchall():
            if pkgKey in pkg_srpm:
                provides_map.setdefault(prov_name, set()).add(pkg_srpm[pkgKey])

        conn.close()

    # 2. Load source packages
    source_packages = {}
    source_deps_raw = {}

    print("Parsing source package metadata and requirements...")
    for key in ['src_baseos', 'src_appstream']:
        conn = sqlite3.connect(dbs[key])
        cursor = conn.cursor()

        cursor.execute('SELECT pkgKey, name, version, release, summary, description, url, rpm_license FROM packages;')
        pkg_keys = {}
        for pkgKey, name, version, release, summary, description, url, license in cursor.fetchall():
            if '%autorelease' in release:
                release = '1'
            if name not in source_packages:
                source_packages[name] = {
                    'name': name,
                    'version': version or '1.0.0',
                    'release': release or '1',
                    'summary': summary or f"CentOS Stream package {name}",
                    'description': description or summary or f"CentOS Stream package {name}",
                    'url': url or f'https://gitlab.com/redhat/centos-stream/rpms/{name}',
                    'license': license or 'GPL',
                    'repository': 'centos-stream'
                }
                pkg_keys[pkgKey] = name

        cursor.execute('SELECT name, pkgKey FROM requires;')
        for req_name, pkgKey in cursor.fetchall():
            if pkgKey in pkg_keys:
                src_name = pkg_keys[pkgKey]
                source_deps_raw.setdefault(src_name, set()).add(req_name)

        conn.close()

    # 3. Resolve dependency graph between source packages
    print("Resolving source-to-source package dependencies...")
    dependency_graph = {}
    for src_name in source_packages:
        deps = set()
        raw_reqs = source_deps_raw.get(src_name, set())
        for req in raw_reqs:
            if req in provides_map:
                for dep_srpm in provides_map[req]:
                    if dep_srpm != src_name and dep_srpm in source_packages:
                        deps.add(dep_srpm)
            elif req in binary_pkg_to_srpm:
                dep_srpm = binary_pkg_to_srpm[req]
                if dep_srpm != src_name and dep_srpm in source_packages:
                    deps.add(dep_srpm)
        dependency_graph[src_name] = sorted(list(deps))

    return source_packages, dependency_graph

class TarjanSCC:
    def __init__(self, graph):
        self.graph = graph
        self.index = 0
        self.indices = {}
        self.lowlinks = {}
        self.on_stack = set()
        self.stack = []
        self.sccs = []

    def solve(self):
        for v in self.graph:
            if v not in self.indices:
                self._strongconnect(v)
        return self.sccs

    def _strongconnect(self, v):
        self.indices[v] = self.index
        self.lowlinks[v] = self.index
        self.index += 1
        self.stack.append(v)
        self.on_stack.add(v)

        for w in self.graph.get(v, []):
            if w not in self.graph:
                continue
            if w not in self.indices:
                self._strongconnect(w)
                self.lowlinks[v] = min(self.lowlinks[v], self.lowlinks[w])
            elif w in self.on_stack:
                self.lowlinks[v] = min(self.lowlinks[v], self.indices[w])

        if self.lowlinks[v] == self.indices[v]:
            scc = []
            while True:
                w = self.stack.pop()
                self.on_stack.remove(w)
                scc.append(w)
                if w == v:
                    break
            self.sccs.append(scc)

def compute_layers_and_cycles(source_packages, dependency_graph):
    # Tarjan's SCC to find cycles and topological order using the dedicated class
    solver = TarjanSCC(dependency_graph)
    sccs = solver.solve()

    # Compute layers
    scc_deps = {}
    pkg_to_scc = {}
    for i, scc in enumerate(sccs):
        for pkg in scc:
            pkg_to_scc[pkg] = i

    for i, scc in enumerate(sccs):
        deps = set()
        for pkg in scc:
            for dep in dependency_graph.get(pkg, []):
                dep_scc = pkg_to_scc[dep]
                if dep_scc != i:
                    deps.add(dep_scc)
        scc_deps[i] = deps

    compilation_layers = []
    remaining_sccs = dict(scc_deps)
    while remaining_sccs:
        layer_sccs = []
        for scc_idx, deps in list(remaining_sccs.items()):
            if all(d not in remaining_sccs for d in deps):
                layer_sccs.append(scc_idx)

        layer_pkgs = []
        for scc_idx in layer_sccs:
            layer_pkgs.extend(sccs[scc_idx])
            del remaining_sccs[scc_idx]
        compilation_layers.append(layer_pkgs)

    return sccs, compilation_layers

def write_sql(dest_sql_path, source_packages, dependency_graph):
    print(f"Writing SQL statements to {dest_sql_path}...")
    sql_statements = []
    sql_statements.append("BEGIN;")
    sql_statements.append("DELETE FROM package_dependencies;")
    sql_statements.append("DELETE FROM package;")

    for name, info in sorted(source_packages.items()):
        v_name = name.replace("'", "''")
        v_version = info['version'].replace("'", "''")
        v_release = info['release'].replace("'", "''")
        v_summary = info['summary'].replace("'", "''")
        v_url = info['url'].replace("'", "''")
        v_license = info['license'].replace("'", "''")
        v_desc = info['description'].replace("'", "''")

        stmt = (
            f"INSERT INTO package (name, version, release, architecture, package_size, source, repository, summary, url, license, description, in_repo, created, vulnerable) "
            f"VALUES ('{v_name}', '{v_version}', '{v_release}', 1, '0', '{v_url}', 'centos-stream', '{v_summary}', '{v_url}', '{v_license}', '{v_desc}', false, false, false);"
        )
        sql_statements.append(stmt)

    sql_statements.append("")
    sql_statements.append("-- Dependencies Inserts")
    for name, deps in sorted(dependency_graph.items()):
        for dep in deps:
            if dep in source_packages:
                stmt = f"INSERT INTO package_dependencies (id_package, id_dependency) VALUES ((SELECT id FROM package WHERE name = '{name}'), (SELECT id FROM package WHERE name = '{dep}'));"
                sql_statements.append(stmt)

    sql_statements.append("COMMIT;")

    with open(dest_sql_path, 'w') as f:
        for s in sql_statements:
            f.write(s + '\n')

def write_report(dest_md_path, dest_sql_path, source_packages, dependency_graph, sccs, compilation_layers):
    print(f"Writing Markdown report to {dest_md_path}...")
    multi_cycles = [scc for scc in sccs if len(scc) > 1]

    with open(dest_md_path, 'w') as f:
        f.write(f"""# CentOS Stream Dependency Graph & Compilation Order

This document details the dependency graph, cycles, and parallel compilation layers computed for all **{len(source_packages)}** source packages.

---

## 1. Summary Statistics

* **Total Source Packages:** {len(source_packages)}
* **Total Dependency Edges:** {sum(len(deps) for deps in dependency_graph.values())}
* **Total Strongly Connected Components (Cycles):** {len(sccs)}
* **Cyclic Components (Size > 1):** {len(multi_cycles)}
* **Total Compilation Layers:** {len(compilation_layers)}

---

## 2. Compilation Layers Breakdown (First 5 Layers)

Below are the packages in the first 5 compilation layers:

""")
        for i, layer in enumerate(compilation_layers[:5]):
            f.write(f"### Layer {i} ({len(layer)} packages)\n")
            f.write(", ".join(sorted(layer)[:50]))
            if len(layer) > 50:
                f.write(f", ... and {len(layer) - 50} more.")
            f.write("\n\n")

        f.write(f"""---

## 3. SQL Insert Statements File

The complete SQL transaction to clear and populate the `package` and `package_dependencies` tables is located at:
👉 **[package_inserts.sql](file://{dest_sql_path})**

To execute this against your PostgreSQL database:
```bash
psql "${{DATABASE_URL}}" -f {dest_sql_path}
```
""")

def main():
    parser = argparse.ArgumentParser(description="Extract CentOS Stream dependency graph and generate SQL inserts.")
    parser.add_argument('--version', type=str, default='10', choices=['9', '10'], help='CentOS Stream version (default: 10)')
    parser.add_argument('--arch', type=str, default='x86_64', help='Target architecture (default: x86_64)')
    parser.add_argument('--output-sql', type=str, default='./package_inserts.sql', help='Output SQL file path')
    parser.add_argument('--output-report', type=str, default='./dependency_graph_and_compilation_order.md', help='Output Markdown report path')

    args = parser.parse_args()

    # Resolve absolute paths
    args.output_sql = os.path.abspath(args.output_sql)
    args.output_report = os.path.abspath(args.output_report)

    with tempfile.TemporaryDirectory() as tmp_dir:
        print(f"Using temp directory: {tmp_dir}")
        source_packages, dependency_graph = analyze_dependencies(tmp_dir, args.version, args.arch)

        if not source_packages or not dependency_graph:
            print("Dependency extraction failed.", file=sys.stderr)
            sys.exit(1)

        print("Analyzing cycles and layers...")
        sccs, compilation_layers = compute_layers_and_cycles(source_packages, dependency_graph)

        # Ensure output directories exist
        os.makedirs(os.path.dirname(args.output_sql), exist_ok=True)
        os.makedirs(os.path.dirname(args.output_report), exist_ok=True)

        write_sql(args.output_sql, source_packages, dependency_graph)
        write_report(args.output_report, args.output_sql, source_packages, dependency_graph, sccs, compilation_layers)

        print("\n=== Analysis Complete ===")
        print(f"Total packages parsed: {len(source_packages)}")
        print(f"Total dependency layers: {len(compilation_layers)}")
        print(f"SQL file written to: {args.output_sql}")
        print(f"Report file written to: {args.output_report}")

if __name__ == '__main__':
    main()
