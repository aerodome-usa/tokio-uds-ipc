{
  description = "tokio-uds-ipc";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
    flake-compat.url = "github:edolstra/flake-compat";

    crane.url = "github:ipetkov/crane";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }@inputs:
    (flake-utils.lib.eachDefaultSystem (
      system:
      let
        bundle =
          targetSystem:
          let
            crossPkgs = import nixpkgs {
              localSystem = system;
              crossSystem = if system == targetSystem then null else targetSystem;
              overlays = [
                rust-overlay.overlays.default
                self.overlays.default
              ];
            };
            hostPkgs = crossPkgs.pkgsBuildHost;
            package = crossPkgs.aerodome.tokio-uds-ipc;
          in
          {
            inherit hostPkgs crossPkgs;
            inherit (package) craneLib;
            checks = hostPkgs.lib.filterAttrs (a: v: !(hostPkgs.lib.hasPrefix "override" a)) (
              crossPkgs.callPackage ./checks.nix {
                inherit package;
                features = [ "default" ];
              }
            );
          };
        mkPackages =
          pkgs:
          let
            pkg = pkgs.aerodome.tokio-uds-ipc;
          in
          {
            tokio-uds-ipc = pkg;
            default = pkg;
            dependencies = pkg.cargoArtifacts;
          };
      in
      nixpkgs.lib.recursiveUpdate
        (
          with (bundle "aarch64-linux");
          nixpkgs.lib.optionalAttrs (system != "aarch64-linux") {
            packagesCross = mkPackages crossPkgs;

            # runs tests with qemu if host != aarch64
            checks = nixpkgs.lib.mapAttrs' (name: value: {
              name = "cross-${name}";
              value = value.overrideAttrs (
                final: prev: {
                  # nixpkgs categorically refuses to run checks if host!=target, so
                  # this is a dirty-dirty hack to force them.
                  postBuild = final.checkPhase;
                }
              );
            }) (nixpkgs.lib.filterAttrs (name: _: nixpkgs.lib.strings.hasPrefix "tests-" name) checks);

            devShells.cross = craneLib.devShell (
              craneLib.mkCrossToolchainEnv (p: p.stdenv)
              // {
                inputsFrom = [ crossPkgs.aerodome.tokio-uds-ipc ];
                packages = with hostPkgs; [
                  cargo-udeps
                  cargo-nextest
                ];
                shellHook = ''
                  configureCargoCommonVars
                  unset CARGO_BUILD_INCREMENTAL CARGO_HOME CARGO_BUILD_JOBS
                '';
              }
            );
          }
        )
        (
          with (bundle system);
          {
            packages = mkPackages hostPkgs;
            inherit checks;
            devShells.default = hostPkgs.mkShell {
              inputsFrom = [ hostPkgs.aerodome.tokio-uds-ipc ];
              packages = with hostPkgs; [
                cargo-udeps
                cargo-nextest
              ];
            };
          }
        )
    ))
    // {
      overlays.default = import ./overlay.nix inputs;
    };
}
