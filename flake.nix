{
  description = "Standalone replacement for systemd-tmpfiles";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable-small";

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;
      makePkgs =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ self.overlays.default ];
        };
      forAllSystems = f: lib.genAttrs lib.systems.flakeExposed (system: f (makePkgs system));
    in
    {
      packages = forAllSystems (pkgs: rec {
        inherit (pkgs) mini-tmpfiles;
        default = mini-tmpfiles;
      });

      devShells = forAllSystems (pkgs: rec {
        mini-tmpfiles =
          with pkgs;
          mkShell {
            packages = [
              rustPackages.cargo
              rustPackages.rustc
              rustPackages.rustfmt
              rustPackages.clippy
            ];
            RUST_SRC_PATH = "${rustPackages.rustPlatform.rustLibSrc}";
          };
        default = mini-tmpfiles;
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-rfc-style);

      overlays.default = final: prev: {
        mini-tmpfiles = final.rustPlatform.buildRustPackage {
          name = "mini-tmpfiles";
          version = "0.1";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          doCheck = false;

          meta = with final.lib; {
            homepage = "https://github.com/nixos-bsd/mini-tmpfiles";
            description = "Standalone replacement for systemd-tmpfiles";
            maintainers = with maintainers; [ artemist ];
            license = with licenses; [ mit ];
            platforms = platforms.unix;
          };
        };
      };
    };
}
