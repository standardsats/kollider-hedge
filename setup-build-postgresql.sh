# Designed for debian
echo "Deploying local PostgreSQL"
pg_ctlcluster 13 main start
sudo -u postgres psql -d postgres -c "create role \"kollider_hedge\" with login password 'kollider_hedge';"
sudo -u postgres psql -d postgres -c "create database \"kollider_hedge\" owner \"kollider_hedge\";"
for f in ./kollider-hedge/migrations/*.sql
do
    echo "Applying $f"
    sudo -u postgres psql -d kollider_hedge -f $f
done
sudo -u postgres psql -d kollider_hedge -c "GRANT ALL PRIVILEGES ON TABLE updates TO kollider_hedge;"
export DATABASE_URL=postgres://kollider_hedge:kollider_hedge@localhost/kollider_hedge
echo "Local database accessible by $DATABASE_URL"
cargo build --release