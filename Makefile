
DB_USER="odoo"
DB_NAME="odoo"
DB_PASSWORD="prueba123"
DB_CONTAINER="dbs_pg"

./data:
	mkdir data

dirs: data

db: data
	@echo "Starting development database container"
	podman run -d --replace --name=$(DB_CONTAINER)                   \
		-e POSTGRES_PASSWORD=$(DB_PASSWORD)                          \
		-e POSTGRES_USER=$(DB_USER)                                  \
		-e POSTGRES_DB=$(DB_NAME)                                    \
		-e POSTGRES_INITDB_ARGS="--encoding UTF-8"                   \
		-v ./data:/var/lib/postgresql/data:U,Z                       \
		-p 127.0.0.1:9436:5432 ghcr.io/enterprisedb/postgresql:18    \
		-c shared_buffers=512MB                                      \
		-c work_mem=64MB                                             \
		-c synchronous_commit=off                                    \
		-c temp_buffers=32MB &&                                      \
		sleep $(SLEEP)

stop-db:
	podman stop $(DB_CONTAINER)

.env:
	echo "DATABASE_URL=postgres://dbs:prueba123!@127.0.0.1:5436/dbs" > .env

release_dirs: bin

clean_db:
	sql/migrate.sh down

bootstrap:
	sql/migrate.sh up

run: .env
	make -C rust/pipeline run

debug:
	make -C rust/pipeline debug

clean:
	make -C rust/pipeline clean

release: release_dirs
	make -C rust/pipeline release

