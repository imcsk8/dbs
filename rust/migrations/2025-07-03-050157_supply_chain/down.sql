BEGIN;

DROP TABLE IF EXISTS package_artifact;
DROP TABLE IF EXISTS package_requires;
DROP TABLE IF EXISTS package_provides;
DROP TABLE IF EXISTS operating_system_package;
DROP TABLE IF EXISTS package_dependencies;
DROP TABLE IF EXISTS product;
DROP TABLE IF EXISTS operating_system;
DROP TABLE IF EXISTS package_manager;
DROP TABLE IF EXISTS package;
DROP TABLE IF EXISTS architecture;
DROP TYPE IF EXISTS build_status CASCADE;
DROP TYPE IF EXISTS os_type CASCADE;
DROP TYPE IF EXISTS os_format CASCADE;

COMMIT;
