{
  # package to run checks against
  package,
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
in
{
  fmt = craneLib.cargoFmt { inherit (package) src; };
  clippy = craneLib.cargoClippy (
    commonArgs
    // {
      cargoClippyExtraArgs = "--all-targets --no-deps -- --deny warnings";
    }
  );
  doc = craneLib.cargoDoc (
    commonArgs
    // {
      RUSTDOCFLAGS = "-D warnings";
      cargoDocExtraArgs = "--no-deps";
    }
  );
  tests = craneLib.cargoNextest (
    commonArgs
    // {
      XDG_RUNTIME_DIR = "/tmp";
      cargoNextestExtraArgs = "--run-ignored all";
      partitions = 1;
      partitionType = "count";
    }
  );
  doctests = craneLib.cargoDocTest commonArgs;
}
