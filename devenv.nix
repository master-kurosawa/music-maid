{
  pkgs,
  lib,
  config,
  inputs,
  ...
}: {
  packages = with pkgs; [sqlx-cli sqlitebrowser openssl opustags vbindiff];
  enterShell = ''
    export DATABASE_URL=sqlite://dev.db
    sqlx database setup
  ''; 
  languages.rust = {
    enable = true;
    channel = "nightly";
  };
}
