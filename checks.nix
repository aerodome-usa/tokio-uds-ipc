{
  # package to run checks against
  package,
  # features to check with
  features,
  # standard
  lib,
}:
let
  inherit (package) craneLib;
  commonArgs = {
    inherit (package)
      src
      buildInputs
      strictDeps
      cargoArtifacts
      ;
    nativeBuildInputs = [ ];
  };
  per-feature = feature: {
    "clippy-${feature}" =
      (craneLib.cargoClippy (
        commonArgs
        // {
          cargoClippyExtraArgs = "--all-targets --no-deps --no-default-features --features ${feature} -- --deny warnings";
        }
      )).overrideAttrs
        (final: prev: { pname = "${prev.pname}-${feature}"; });
    "doc-${feature}" =
      (craneLib.cargoDoc (
        commonArgs
        // {
          RUSTDOCFLAGS = "-D warnings";
          cargoDocExtraArgs = "--no-deps --no-default-features --features ${feature}";
        }
      )).overrideAttrs
        (final: prev: { pname = "${prev.pname}-${feature}"; });
    "tests-${feature}" =
      (craneLib.cargoNextest (
        commonArgs
        // {
          XDG_RUNTIME_DIR = "/tmp";
          cargoNextestExtraArgs = "--no-default-features --features ${feature} --run-ignored all";
          partitions = 1;
          partitionType = "count";
        }
      )).overrideAttrs
        (final: prev: { pname = "${prev.pname}-${feature}"; });
    "doctests-${feature}" =
      (craneLib.cargoDocTest (
        commonArgs
        // {
          cargoTestExtraArgs = "--no-default-features --features ${feature}";
        }
      )).overrideAttrs
        (final: prev: { pname = "${prev.pname}-${feature}"; });
  };
in
lib.foldl' lib.recursiveUpdate {
  fmt = craneLib.cargoFmt { inherit (package) src; };
} (lib.map per-feature features)
