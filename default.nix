let
  sources = import ./nix/sources.nix;
  nixpkgs-mozilla = import sources.nixpkgs-mozilla;
  pkgs = import sources.nixpkgs {
    overlays =
      [
        nixpkgs-mozilla
        (self: super:
            let chan = self.rustChannelOf { date = "2022-01-25"; channel = "nightly"; };
            in {
              rustc = chan.rust;
              cargo = chan.rust;
            }
        )
      ];
  };
  naersk = pkgs.callPackage sources.naersk {};
  merged-openssl = pkgs.symlinkJoin { name = "merged-openssl"; paths = [ pkgs.openssl.out pkgs.openssl.dev ]; };
in
naersk.buildPackage {
  name = "kollider-hedge";
  root = pkgs.lib.sourceFilesBySuffices ./. [".rs" ".toml" ".lock" ".html" ".css" ".png" ".sh" ".sql"];
  buildInputs = with pkgs; [ openssl pkgconfig clang llvm llvmPackages.libclang zlib cacert curl postgresql ];
  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang}/lib";
  OPENSSL_DIR = "${merged-openssl}";
  preBuild = ''
    echo "Deploying local PostgreSQL"
    initdb ./pgsql-data --auth=trust
    echo "unix_socket_directories = '$PWD'" >> ./pgsql-data/postgresql.conf
    pg_ctl start -D./pgsql-data -l psqlog
    psql --host=$PWD -d postgres -c "create role \"kollider_hedge\" with login password 'kollider_hedge';"
    psql --host=$PWD -d postgres -c "create database \"kollider_hedge\" owner \"kollider_hedge\";"
    cp -r ${./kollider-hedge/migrations} ./kollider-hedge/migrations
    for f in ./kollider-hedge/migrations/*.sql
    do
      echo "Applying $f"
      psql --host=$PWD -U kollider_hedge -d kollider_hedge -f $f
    done
    export DATABASE_URL=postgres://kollider_hedge:kollider_hedge@localhost/kollider_hedge
    echo "Local database accessible by $DATABASE_URL"
  '';
}
