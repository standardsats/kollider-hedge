# How to build (development)

You need to run local PostgreSQL instance to allow compiler to check SQL quieries in advance:
1. Create `kollider` user with `kollider` password.
2. Allow database creation for the user. That is required for temporary databases for tests.
3. Create `kollider_hedge` database and add `kollider` as owner.
4. Set env variable with connection string `DATABASE_URL=postgresql://kollider:kollider@localhost:5432/kollider_hedge`
5. Run initial migrations with the following command:
```
cargo run --bin kollider-hedge-db migrate
```
6. Run build (don't forget to set `DATABASE_URL` at the step 3):
```
cargo build
```
7. Run tests:
```
cargo test
```

# How to run
Provide the connection URL `KOLLIDER_HEDGE_POSTGRES` environment or pass it via `dbconnect` argument
```
KOLLIDER_HEDGE_POSTGRES=postgresql://kollider:kollider@localhost:5432/kollider_hedge kollider-hedge serve
```

Alsow you can run CLI to access API of the plugin from the terminal:
```
kollider-hedge-cli --help
```

# Docker

You can build the Docker images either by Nix or Docker:
- Nix: `./make-docker.sh`. The result image will be saved to `docker-image-kollider-hedge.tar.gz`. You can load it with:
```
docker load < ./docker-image-kollider-hedge.tar.gz
```
- Docker: `docker build -t kollider-hedge .`

Nix will produce more compact images (4 Kb agains 35 Mb) and doesn't require Docker daemon installed to build.

Next, you can run the image with docker compose. Put kollider secrets into the `.env` file:
```
KOLLIDER_API_KEY="*******"
KOLLIDER_API_SECRET="*******"
KOLLIDER_API_PASSWORD="*********"
```
Run: `docker-compose up`
