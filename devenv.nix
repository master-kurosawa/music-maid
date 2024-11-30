{
  pkgs,
  inputs,
  ...
}: {
  packages = with pkgs; [
    inputs.mbslave.packages.${system}.default
    sqlx-cli
    protobuf_26
    postgresql
  ];

  languages.typescript.enable = true;
  languages.javascript = {
    enable = true;
    bun.enable = true;
  };

  languages.rust = {
    enable = true;
    channel = "nightly";
  };

  # for mbslave
  services.postgres = {
    enable = true;
    initialScript = ''
      CREATE ROLE postgres WITH LOGIN SUPERUSER PASSWORD 'postgres';
    '';
  };
  dotenv.enable = true;

  enterShell = ''
    export DATABASE_URL=postgres:///musicbrainz?host=$PGHOST
    export PROTOBUF_LOCATION=${pkgs.protobuf_26}
    export PROTOC=$PROTOBUF_LOCATION/bin/protoc
    export PROTOC_INCLUDE=$PROTOBUF_LOCATION/include
    export OUT_DIR=$DEVENV_ROOT/target/
  '';
}
