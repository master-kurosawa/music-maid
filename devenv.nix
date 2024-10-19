{
  pkgs,
  lib,
  config,
  inputs,
  ...
}: {
  packages = with pkgs; [sqlitebrowser openssl];
  languages.rust = {
    enable = true;
    channel = "nightly";
  };
}
