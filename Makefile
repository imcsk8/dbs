
DB_USER="dbs"
DB_NAME="dbs"
DB_PASSWORD="prueba123"
DB_CONTAINER="dbs_pg"

./data:
	mkdir data

dirs: data
	mkdir -p bin mock data/distgit staging

db: data
	@echo "Starting development database container"
	podman run -d --replace --name=$(DB_CONTAINER)                   \
		-e POSTGRES_PASSWORD=$(DB_PASSWORD)                          \
		-e POSTGRES_USER=$(DB_USER)                                  \
		-e POSTGRES_DB=$(DB_NAME)                                    \
		-e POSTGRES_INITDB_ARGS="--encoding UTF-8"                   \
		-v ./data:/var/lib/postgresql/data:U,Z                       \
		-p 127.0.0.1:9436:5432 ghcr.io/enterprisedb/postgresql:17    \
		-c shared_buffers=512MB                                      \
		-c work_mem=64MB                                             \
		-c synchronous_commit=off                                    \
		-c temp_buffers=32MB &&                                      \
		sleep $(SLEEP)

stop-db:
	podman stop $(DB_CONTAINER)

.env:
	echo "DATABASE_URL=postgres://dbs:prueba123@127.0.0.1:9436/dbs" > .env

bin:
	mkdir -p bin

release_dirs: bin

clean_db:
	@if [ -x ./bin/dbs ]; then ./bin/dbs db reset --force; else cd sql && ./migrate.sh down; fi

bootstrap:
	@if [ -x ./bin/dbs ]; then ./bin/dbs db bootstrap; else cd sql && ./migrate.sh up; fi


build:
	make -C rust build

test:
	make -C rust test

run: .env
	make -C rust run

debug:
	make -C rust debug

clean:
	make -C rust clean

release: release_dirs
	make -C rust release
	cp rust/bin/dbs bin/dbs

