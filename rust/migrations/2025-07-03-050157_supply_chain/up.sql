BEGIN;

DROP TYPE IF EXISTS build_status CASCADE;
CREATE TYPE build_status AS
ENUM ('PENDING', 'BUILDING', 'SUCCESS', 'FAILED', 'SKIPPED');

DROP TYPE IF EXISTS os_type CASCADE;
CREATE TYPE os_type AS
ENUM ('VERSIONED', 'ROLLING');

DROP TYPE IF EXISTS os_format CASCADE;
CREATE TYPE os_format AS
ENUM ('ISO', 'OCI', 'QCOW', 'RAW');



-- Package Manager
CREATE TABLE IF NOT EXISTS package_manager (
	id SERIAL PRIMARY KEY,
	name TEXT NOT NULL,
	description TEXT
);
COMMENT ON TABLE package_manager IS E'Distribution package manager';

COMMENT ON COLUMN package_manager.name IS E'Name of the package manager';
COMMENT ON COLUMN package_manager.description IS E'Description of the package manager';

CREATE INDEX IF NOT EXISTS idx_pm_name ON package_manager
USING btree
(
    name
);

-- Inserts for the package_manager table

-- RPM for Red Hat-based distributions (Red Hat Enterprise Linux, Fedora, CentOS, TacOS)
INSERT INTO package_manager (name, description) VALUES 
('rpm', 'The RPM Package Manager, used by Red Hat-based distributions. It is a powerful command-line utility for installing, uninstalling, verifying, querying, and updating software packages.');

-- Dpkg for Debian-based distributions (Debian, Ubuntu, Mint)
INSERT INTO package_manager (name, description) VALUES 
('dpkg', 'The low-level package manager for Debian-based systems. It can install, remove, and build packages, but unlike higher-level tools, it does not automatically handle dependencies.');

-- Pacman for Arch Linux and its derivatives
INSERT INTO package_manager (name, description) VALUES 
('pacman', 'The package manager for Arch Linux. It combines a simple binary package format with an easy-to-use build system, synchronizing packages from a master server to keep the system up-to-date.');

-- Pkg for FreeBSD and its derivatives
INSERT INTO package_manager (name, description) VALUES 
('pkg', 'The package management tool for FreeBSD. It is used for installing, upgrading, and removing binary packages, and it handles dependencies automatically.');

-- Portage for Gentoo Linux
INSERT INTO package_manager (name, description) VALUES 
('portage', 'The package manager and distribution system for Gentoo. It is based on the concept of "ports" collections and compiles packages from source code according to user-specified "USE flags".');

-- eopkg for Solus
INSERT INTO package_manager (name, description) VALUES
('eopkg', 'The package manager for the Solus operating system. It is a fork of the PiSi package manager.');

-- apk for Alpine Linux
INSERT INTO package_manager (name, description) VALUES
('apk', 'The Alpine Package Keeper, the package manager for Alpine Linux. It is designed to be small, simple, and secure.');


-- Architecture
CREATE TABLE IF NOT EXISTS architecture (
	id SERIAL PRIMARY KEY,
	name TEXT NOT NULL,
	description TEXT
);
COMMENT ON TABLE architecture IS E'Operating system architecture';

COMMENT ON COLUMN architecture.name IS E'Architecture name';
COMMENT ON COLUMN architecture.description IS E'Architecture description';

CREATE INDEX IF NOT EXISTS idx_architecture_name ON architecture
USING btree
(
    name
);

INSERT INTO architecture (name, description) VALUES
('x86_64', 'AMD/Intel 64-bit architecture, commonly used in desktops, servers, and cloud environments. It is the most prevalent architecture.'),
('aarch64', 'The 64-bit execution state of the ARM architecture, also known as ARM64. Widely used in mobile devices, and increasingly in servers and edge computing.'),
('ppc64le', 'IBM Power Little Endian, a 64-bit architecture used in enterprise-level servers and for high-performance computing (HPC) workloads.'),
('s390x', 'The 64-bit architecture for IBM Z mainframes, designed for high-volume transaction processing and enterprise-scale applications.'),
('RISC-V', 'An open-source instruction set architecture (ISA). Support in CentOS Stream and Fedora is in early stages, offered as a developer preview for specific hardware.');


-- Package
CREATE TABLE IF NOT EXISTS package (
    id SERIAL PRIMARY KEY,
	name TEXT NOT NULL,
	epoch INT NOT NULL DEFAULT 0,
	version TEXT NOT NULL,
	release TEXT NOT NULL,
    architecture INT NOT NULL DEFAULT 1 REFERENCES architecture(id) ON DELETE CASCADE,
	package_size TEXT NOT NULL,
	file_size_bytes BIGINT NOT NULL DEFAULT 0,
	source TEXT NOT NULL,
	repository TEXT NOT NULL,
	summary TEXT NOT NULL,
	url TEXT NOT NULL,
	license TEXT NOT NULL,
	description TEXT NOT NULL,
	in_repo BOOLEAN DEFAULT FALSE,
	created BOOLEAN DEFAULT FALSE,
	vulnerable BOOLEAN DEFAULT FALSE,
	build_status build_status NOT NULL DEFAULT 'PENDING',
	build_duration_seconds REAL,
	build_log_path TEXT,
	error_summary TEXT,
	worker_id INT,
	sourcerpm TEXT,
	dist_git_url TEXT,
	dist_git_branch TEXT,
	dist_git_commit TEXT,
	spec_file TEXT
);
COMMENT ON TABLE package IS E'A package that integrates into the distribution';

COMMENT ON COLUMN package.name IS E'Name of the package';
COMMENT ON COLUMN package.epoch IS E'Package epoch for EVR comparison';
COMMENT ON COLUMN package.version IS E'Package version';
COMMENT ON COLUMN package.release IS E'Package release';
COMMENT ON COLUMN package.architecture IS E'Package architecture';
COMMENT ON COLUMN package.package_size IS E'Package size human readable';
COMMENT ON COLUMN package.file_size_bytes IS E'Package file size in bytes';
COMMENT ON COLUMN package.source IS E'Package source URL or archive';
COMMENT ON COLUMN package.repository IS E'Package repository identifier';
COMMENT ON COLUMN package.summary IS E'Package summary';
COMMENT ON COLUMN package.url IS E'Package upstream project URL';
COMMENT ON COLUMN package.license IS E'Package license';
COMMENT ON COLUMN package.description IS E'Package description';
COMMENT ON COLUMN package.in_repo IS E'True if package has been published to a repository';
COMMENT ON COLUMN package.created IS E'True if the package has been compiled';
COMMENT ON COLUMN package.vulnerable IS E'True if there are security advisories that have not been fixed';
COMMENT ON COLUMN package.build_status IS E'Lifecycle state: PENDING, BUILDING, SUCCESS, FAILED, SKIPPED';
COMMENT ON COLUMN package.build_duration_seconds IS E'Build duration in seconds';
COMMENT ON COLUMN package.build_log_path IS E'Filesystem path to build log';
COMMENT ON COLUMN package.error_summary IS E'Error or failure snippet if build failed';
COMMENT ON COLUMN package.worker_id IS E'Identifier of worker that built the package';
COMMENT ON COLUMN package.sourcerpm IS E'Corresponding source RPM filename';
COMMENT ON COLUMN package.dist_git_url IS E'Dist-git repository clone URL';
COMMENT ON COLUMN package.dist_git_branch IS E'Dist-git branch name';
COMMENT ON COLUMN package.dist_git_commit IS E'Dist-git commit hash';
COMMENT ON COLUMN package.spec_file IS E'Relative path to spec file in dist-git';

CREATE INDEX IF NOT EXISTS idx_name ON package
USING btree
(
    name
);

CREATE INDEX IF NOT EXISTS idx_package_repo ON package
USING btree
(
    in_repo
);

CREATE INDEX IF NOT EXISTS idx_package_created ON package
USING btree
(
    created
);

CREATE INDEX IF NOT EXISTS idx_package_build_status ON package
USING btree
(
    build_status
);

CREATE INDEX IF NOT EXISTS idx_package_sourcerpm ON package
USING btree
(
    sourcerpm
);


-- Package provides capability
CREATE TABLE IF NOT EXISTS package_provides (
    id SERIAL PRIMARY KEY,
    id_package INT NOT NULL REFERENCES package(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    flags TEXT,
    version TEXT
);
COMMENT ON TABLE package_provides IS E'Capabilities provided by a package';

CREATE INDEX IF NOT EXISTS idx_pkg_provides_name ON package_provides
USING btree
(
    name
);

CREATE INDEX IF NOT EXISTS idx_pkg_provides_pkg ON package_provides
USING btree
(
    id_package
);


-- Package requires capability
CREATE TABLE IF NOT EXISTS package_requires (
    id SERIAL PRIMARY KEY,
    id_package INT NOT NULL REFERENCES package(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    flags TEXT,
    version TEXT,
    is_build_require BOOLEAN NOT NULL DEFAULT FALSE
);
COMMENT ON TABLE package_requires IS E'Capabilities required by a package (runtime or build-time)';

CREATE INDEX IF NOT EXISTS idx_pkg_requires_name ON package_requires
USING btree
(
    name
);

CREATE INDEX IF NOT EXISTS idx_pkg_requires_pkg ON package_requires
USING btree
(
    id_package
);

CREATE INDEX IF NOT EXISTS idx_pkg_requires_build ON package_requires
USING btree
(
    is_build_require
);


-- Package built artifacts
CREATE TABLE IF NOT EXISTS package_artifact (
    id SERIAL PRIMARY KEY,
    id_package INT REFERENCES package(id) ON DELETE CASCADE,
    rpm_filename TEXT NOT NULL,
    rpm_path TEXT NOT NULL,
    arch TEXT NOT NULL,
    is_source BOOLEAN NOT NULL DEFAULT FALSE,
    file_size_bytes BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);
COMMENT ON TABLE package_artifact IS E'Compiled binary and source package artifacts';

CREATE INDEX IF NOT EXISTS idx_artifact_pkg ON package_artifact
USING btree
(
    id_package
);

CREATE INDEX IF NOT EXISTS idx_artifact_arch ON package_artifact
USING btree
(
    arch
);


-- OS
CREATE TABLE IF NOT EXISTS operating_system (
    id SERIAL PRIMARY KEY,
	name TEXT NOT NULL,
	version TEXT NOT NULL,
	system_type os_type NOT NULL DEFAULT 'VERSIONED',
	release TEXT NOT NULL,
    architecture INT NOT NULL DEFAULT 1 REFERENCES architecture(id) ON DELETE CASCADE,
	summary TEXT NOT NULL,
	url TEXT NOT NULL,
	license TEXT NOT NULL,
	description TEXT NOT NULL,
    manager INT NOT NULL DEFAULT 1 REFERENCES package_manager(id) ON DELETE CASCADE,
    distro_tag TEXT,
    dist_git_url_template TEXT,
    dist_git_branch TEXT,
    lookaside_cache_url TEXT,
    api_type TEXT DEFAULT 'pagure',
    api_url TEXT,
    mock_chroot TEXT
);
COMMENT ON TABLE operating_system IS E'Entity that integrates the packages';

COMMENT ON COLUMN operating_system.name IS E'Name of the operating system';
COMMENT ON COLUMN operating_system.version IS E'Operating system version';
COMMENT ON COLUMN operating_system.system_type IS E'Operating system type: VERSIONED or ROLLING';
COMMENT ON COLUMN operating_system.release IS E'Version of the operating system with id metadata';
COMMENT ON COLUMN operating_system.architecture IS E'Operating system architecture: x86, x86_64, ARM, etc...';
COMMENT ON COLUMN operating_system.summary IS E'Operating system summary';
COMMENT ON COLUMN operating_system.url IS E'Operating system URL';
COMMENT ON COLUMN operating_system.license IS E'Operating system license';
COMMENT ON COLUMN operating_system.description IS E'Operating system description';
COMMENT ON COLUMN operating_system.manager IS E'Package manager the OS is based on (rpm, dpkg, etc...)';
COMMENT ON COLUMN operating_system.distro_tag IS E'Distribution tag eg: el10';
COMMENT ON COLUMN operating_system.dist_git_url_template IS E'Template for dist-git clone URL (supports {package} placeholder)';
COMMENT ON COLUMN operating_system.dist_git_branch IS E'Default branch for dist-git repositories';
COMMENT ON COLUMN operating_system.lookaside_cache_url IS E'Lookaside cache base URL for large source tarballs';
COMMENT ON COLUMN operating_system.api_type IS E'Dist-git API type: pagure, gitlab, cgit, forgejo';
COMMENT ON COLUMN operating_system.api_url IS E'Base API endpoint for discovering projects and packages';
COMMENT ON COLUMN operating_system.mock_chroot IS E'Default Mock chroot configuration profile name';

CREATE INDEX IF NOT EXISTS idx_os_name ON operating_system
USING btree
(
    name
);

CREATE INDEX IF NOT EXISTS idx_os_architecture ON operating_system
USING btree
(
    architecture
);

INSERT INTO operating_system (name, version, system_type, release, architecture, summary, url, license, description, manager, distro_tag, dist_git_url_template, dist_git_branch, lookaside_cache_url, api_type, api_url, mock_chroot) VALUES
('CentOS Stream', '10', 'VERSIONED', '10-stream', 1, 'CentOS Stream 10 distribution', 'https://centos.org', 'GPL', 'Upstream development platform for Red Hat Enterprise Linux 10', 1, 'el10', 'https://gitlab.com/redhat/centos-stream/rpms/{package}.git', 'c10s', 'https://sources.stream.centos.org/sources/rpms', 'gitlab', 'https://gitlab.com/api/v4/groups/8794173/projects', 'centos-stream-10-x86_64'),
('CentOS Stream', '9', 'VERSIONED', '9-stream', 1, 'CentOS Stream 9 distribution', 'https://centos.org', 'GPL', 'Upstream development platform for Red Hat Enterprise Linux 9', 1, 'el9', 'https://gitlab.com/redhat/centos-stream/rpms/{package}.git', 'c9s', 'https://sources.stream.centos.org/sources/rpms', 'gitlab', 'https://gitlab.com/api/v4/groups/8794173/projects', 'centos-stream-9-x86_64'),
('Fedora', 'Rawhide', 'ROLLING', 'rawhide', 1, 'Fedora Rawhide development distribution', 'https://fedoraproject.org', 'GPL', 'Continuous development branch of Fedora Linux', 1, 'fc42', 'https://src.fedoraproject.org/rpms/{package}.git', 'rawhide', 'https://src.fedoraproject.org/repo/pkgs', 'pagure', 'https://src.fedoraproject.org/api/0', 'fedora-rawhide-x86_64'),
('TacOS', 'Rolling', 'ROLLING', 'rolling', 1, 'TacOS proof-of-concept rolling enterprise OS', 'https://tacos.org.mx', 'GPL-3.0-or-later', 'Bleeding-edge enterprise rolling release based on Fedora ELN and Rawhide', 1, 'tcrs', 'https://codeberg.org/imcsk8/tacos.git', 'master', 'https://repos.tacos.org.mx/sources', 'forgejo', 'https://codeberg.org/api/v1', 'tacos-rolling-x86_64');


-- Operating system package relation
CREATE TABLE operating_system_package (
    id SERIAL PRIMARY KEY,
    id_os INT REFERENCES operating_system(id) ON DELETE CASCADE,
    id_package INT REFERENCES package(id) ON DELETE CASCADE
);
COMMENT ON TABLE operating_system_package IS E'Which packages belong to the operating system';

COMMENT ON COLUMN operating_system_package.id_os IS E'Operating system ID';
COMMENT ON COLUMN operating_system_package.id_package IS E'Package ID';

CREATE INDEX IF NOT EXISTS idx_os_package ON operating_system_package
USING btree
(
    id_os,
	id_package
);


-- Package dependencies
CREATE TABLE package_dependencies (
    id SERIAL PRIMARY KEY,
    id_package INT REFERENCES package(id) ON DELETE CASCADE,
    id_dependency INT REFERENCES package(id) ON DELETE CASCADE
);
COMMENT ON TABLE package_dependencies IS E'Resolved package-level DAG build order dependencies';

COMMENT ON COLUMN package_dependencies.id_package IS E'Package ID';
COMMENT ON COLUMN package_dependencies.id_dependency IS E'Dependent Package ID';

CREATE INDEX IF NOT EXISTS idx_package_deps_os_package ON package_dependencies
USING btree
(
	id_package,
    id_dependency
);

-- Product
CREATE TABLE IF NOT EXISTS product (
    id SERIAL PRIMARY KEY,
	id_os INT NOT NULL DEFAULT 1 REFERENCES operating_system(id) ON DELETE CASCADE,
	format os_format NOT NULL DEFAULT 'ISO'
);
COMMENT ON TABLE product IS E'operating system in a specific format';

COMMENT ON COLUMN product.id_os IS E'ID of the operating system';
COMMENT ON COLUMN product.format IS E'Format of the operating system: ISO, qcow, etc...';

CREATE INDEX IF NOT EXISTS idx_product_format ON product
USING btree
(
    format
);


COMMIT;
