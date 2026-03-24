final: prev: {
  mini-tmpfiles = final.rustPlatform.buildRustPackage {
    name = "mini-tmpfiles";
    version = "0.1";
    src =
      with final.lib.fileset;
      toSource {
        root = ./.;
        fileset = unions [
          ./src
          ./Cargo.toml
          ./Cargo.lock
        ];
      };
    cargoLock.lockFile = ./Cargo.lock;

    meta = with final.lib; {
      homepage = "https://github.com/nixos-bsd/mini-tmpfiles";
      description = "Standalone replacement for systemd-tmpfiles";
      maintainers = with maintainers; [ artemist ];
      license = with licenses; [ mit ];
      platforms = platforms.unix;
    };
  };
}
