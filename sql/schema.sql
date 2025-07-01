DROP TYPE IF EXISTS os_type CASCADE;
CREATE TYPE os_type AS
ENUM ('VERSIONED', 'ROLLING');

DROP TYPE IF EXISTS os_format CASCADE;
CREATE TYPE os_format AS
ENUM ('ISO', 'OCI', 'QCOW', 'RAW');


-- Product
COMMENT ON TABLE product IS E'operating system in a specific format';
CREATE TABLE IF NOT EXISTS product (
    id SERIAL PRIMARY KEY,
	id_os INT NOT NULL DEFAULT 1 REFERENCES operating_system(id) ON DELETE CASCADE,
	format os_format NOT NULL DEFAULT 'ISO',
)

COMMENT ON COLUMN product.id_os IS E'ID of the operating system';
COMMENT ON COLUMN product.format IS E'Format of the operating system: ISO, qcow, etc...';


CREATE INDEX IF NOT EXISTS idx_product_format ON product
USING btree
(
    format
);

-- OS
COMMENT ON TABLE operating_system IS E'Entity that integrates the genrated packages';
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
	manager package_manager NOT NULL DEFAULT 'rpm'
)

COMMENT ON COLUMN operating_system.name IS E'Name of the operating system';
COMMENT ON COLUMN operating_system.version IS E'Operating system version';
COMMENT ON COLUMN operating_system.system_type IS E'Operating system type: VERSIONED or ROLLING';
COMMENT ON COLUMN operating_system.release IS E'Version of the operating system with id metadata';
COMMENT ON COLUMN operating_system.architecture IS E'Operating system architecture: x86, x86_64, ARM, etc...';
COMMENT ON COLUMN operating_system.summary IS E'Operating system summary';
COMMENT ON COLUMN operating_system.url IS E'Operating system URL';
COMMENT ON COLUMN operating_system.license IS E'Operating system license';
COMMENT ON COLUMN operating_system.description IS E'Operating system description';
COMMENT ON COLUMN operating_system.manager IS E'Package manage the OS is based on (rpm, dpkg, etc...)';

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


-- Package Manager
COMMENT ON TABLE package_manager IS E'Distribution package manager';
CREATE TABLE IF NOT EXISTS package_manager (
	id SERIAL PRIMARY KEY,
	name TEXT NOT NULL,
	description TEXT,
);


COMMENT ON COLUMN package_manager.name IS E'Name of the package manager';
COMMENT ON COLUMN package_manager.description IS E'Description of the package manager';

CREATE INDEX IF NOT EXISTS idx_pm_name ON package_manager
USING btree
(
    name
);

-- Inserts for the package_manager table

-- RPM for Red Hat-based distributions (Red Hat Enterprise Linux, Fedora, CentOS)
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
COMMENT ON TABLE architecture IS E'Operating system architecture';
CREATE TABLE IF NOT EXISTS architecture (
	id SERIAL PRIMARY KEY,
	name TEXT NOT NULL,
	description TEXT,
);

COMMENT ON COLUMN architecture.name IS E'Architecture name';
COMMENT ON COLUMN architecture.description IS E'Architecture description';

CREATE INDEX IF NOT EXISTS idx_architecture_name ON operating_system
USING btree
(
    name
);


INSERT INTO architecture (name, description) VALUES
('x86_64', 'AMD/Intel 64-bit architecture, commonly used in desktops, servers, and cloud environments. It is the most prevalent architecture.'),
('aarch64', 'The 64-bit execution state of the ARM architecture, also known as ARM64. Widely used in mobile devices, and increasingly in servers and edge computing.'),
('ppc64le', 'IBM Power Little Endian, a 64-bit architecture used in enterprise-level servers and for high-performance computing (HPC) workloads.'),
('s390x', 'The 64-bit architecture for IBM Z mainframes, designed for high-volume transaction processing and enterprise-scale applications.'),
('RISC-V', 'An open-source instruction set architecture (ISA). Support in CentOS Stream is in its early stages, offered as a developer preview for specific hardware.');


-- Package
COMMENT ON TABLE package_manager IS E'A package that integrates into the distribution';
CREATE TABLE IF NOT EXISTS package (
    id SERIAL PRIMARY KEY,
	name TEXT NOT NULL,
	version TEXT NOT NULL,
	release TEXT NOT NULL,
    architecture INT NOT NULL DEFAULT 1 REFERENCES architecture(id) ON DELETE CASCADE,
	package_size TEXT NOT NULL,
	source TEXT NOT NULL,
	repository TEXT NOT NULL,
	summary TEXT NOT NULL,
	url TEXT NOT NULL,
	license TEXT NOT NULL,
	description TEXT NOT NULL,
	in_repo bool,   -- The package has been sent to a repository
	created bool,   -- The package has been created
	vulnerable bool -- Security advisories not addressed
)

COMMENT ON COLUMN package_manager.name IS E'Name of the package';
COMMENT ON COLUMN package_manager.version IS E'Package version';
COMMENT ON COLUMN package_manager.release IS E'Package release';
COMMENT ON COLUMN package_manager.architecture IS E'Package architecture';
COMMENT ON COLUMN package_manager.package_size IS E'Package size';
COMMENT ON COLUMN package_manager.source IS E'Package source URL';
COMMENT ON COLUMN package_manager.repository IS E'Package repository';
COMMENT ON COLUMN package_manager.summary IS E'Package summary';
COMMENT ON COLUMN package_manager.url IS E'Package URL';
COMMENT ON COLUMN package_manager.license IS E'Package license';
COMMENT ON COLUMN package_manager.description IS E'Package description';
COMMENT ON COLUMN package_manager.in_repo IS E'True if package has been uploaded to a repo';
COMMENT ON COLUMN package_manager.created IS E'True if the package has been created';
COMMENT ON COLUMN package_manager.vulnerable IS E'True if there are security advisories that have not been fixed';

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



-- Operating system package relation
COMMENT ON TABLE operating_system_package IS E'Which packages belong to the operating system';
CREATE TABLE operating_system_package (
    id SERIAL PRIMARY KEY,
    id_os INT REFERENCES operating_system(id) ON DELETE CASCADE,
    id_package INT REFERENCES package(id) ON DELETE CASCADE,
)

COMMENT ON COLUMN operating_system_package.id_os IS E'Operating system ID';
COMMENT ON COLUMN operating_system_package.id_package IS E'Package ID';

CREATE INDEX IF NOT EXISTS idx_os_package ON operating_system_package
USING btree
(
    id_os,
	id_package
);


-- Package dependencies
COMMENT ON TABLE package_dependencies IS E'Dependencies of a package';
CREATE TABLE package_dependencies (
    id SERIAL PRIMARY KEY,
    id_package INT REFERENCES package(id) ON DELETE CASCADE,
    id_dependency INT REFERENCES package(id) ON DELETE CASCADE,
)


COMMENT ON COLUMN package_dependencies.id_package IS E'Package ID';
COMMENT ON COLUMN package_dependencies.id_dependency IS E'Dependent Package ID';

CREATE INDEX IF NOT EXISTS idx_os_package ON operating_system_package
USING btree
(
    id_os,
	id_package
);

