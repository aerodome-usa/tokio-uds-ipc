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
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            rust-overlay.overlays.default
            self.overlays.default
          ];
        };
      in
      {
        packages = {
          tokio-uds-ipc = pkgs.aerodome.tokio-uds-ipc;
          default = pkgs.aerodome.tokio-uds-ipc;
          dependencies = pkgs.aerodome.tokio-uds-ipc.cargoArtifacts;
        };
        checks = pkgs.lib.filterAttrs (a: v: !(pkgs.lib.hasPrefix "override" a)) (
          pkgs.callPackage ./checks.nix {
            package = pkgs.aerodome.tokio-uds-ipc;
          }
        );
        devShells.default = pkgs.mkShell {
          inputsFrom = [ pkgs.aerodome.tokio-uds-ipc ];
          packages = with pkgs; [
            cargo-udeps
            cargo-nextest
          ];
        };
        check-matrix =
          let
            mkMatrix = name: attrs: {
              include = map (v: { ${name} = v; }) (pkgs.lib.attrNames attrs);
            };
          in
          mkMatrix "check" self.checks.${system};
      }
    ))
    // {
      overlays.default = import ./overlay.nix inputs;
    };
}
