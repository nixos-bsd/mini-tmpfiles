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
      checks = forAllSystems (pkgs: {
        mini-vmtest = pkgs.callPackage ./tests/test.nix { };
        reference-test = pkgs.callPackage ./tests/test.nix { reference = true; };
      });
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

      overlays.default = import ./overlay.nix; 
    };
}
