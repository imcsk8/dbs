
.env:
	echo "DATABASE_URL=postgres://dbs:prueba123!@127.0.0.1:5432/dbs" > .env

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

