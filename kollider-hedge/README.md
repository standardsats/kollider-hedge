# How to build
You need to run local PostgreSQL instance to allow compiler to check SQL quieries in advance:
1. Create `kollider` user with `kollider` password.
2. Allow database creation for the user. That is required for temporary databases for tests. `ALTER USER kollider CREATEDB;`
3. Create `kollider_hedge` database and add `kollider` as owner.

Run build:
```
DATABASE_URL=postgresql://kollider:kollider@localhost:5432/kollider_hedge cargo build
```

Run tests:
```
DATABASE_URL=postgresql://kollider:kollider@localhost:5432/kollider_hedge cargo test
```

Next, I will assume that `DATABASE_URL` is accessible by cargo commands.

# Swagger

The binary can produce OpenAPI specification:
```
cargo run -- swagger
```