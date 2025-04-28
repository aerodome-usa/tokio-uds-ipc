{
  # crane flake
  crane,
  toolchainOverride ? null,
  # standard
  pkgs,
  lib,
}:
let
  nativeBuildInputs = [ ];
  buildInputs = [ ];
  toolchain = lib.defaultTo (
    pkgs:
    (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override {
      extensions = [ "rust-src" ];
    }
  ) toolchainOverride;
  craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
  src = lib.cleanSourceWith {
    src = craneLib.path ./.;
    filter = craneLib.filterCargoSources;
  };
  commonArgsNoDeps = {
    inherit src;
    strictDeps = true;
    inherit nativeBuildInputs buildInputs;
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgsNoDeps;
  commonArgs = commonArgsNoDeps // {
    inherit cargoArtifacts;
  };
in
craneLib.buildPackage (commonArgs)
// {
  inherit craneLib;
}
