{ crane, ... }:
final: prev: {
  aerodome = (prev.aerodome or (final.lib.makeScope final.newScope (_: { }))).overrideScope (
    final: prev: {
      tokio-uds-ipc = final.callPackage ./default.nix {
        inherit crane;
      };
    }
  );
}
