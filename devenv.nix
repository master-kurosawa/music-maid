{
  pkgs,
  lib,
  config,
  inputs,
  ...
}: {
  packages = [
    inputs.mbslave.packages.${pkgs.system}.default
    pkgs.pgadmin4-desktopmode
    pkgs.sqlx-cli
  ];

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
  '';
}
