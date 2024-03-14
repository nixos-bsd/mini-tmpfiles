{
  description = "Standalone replacement for systemd-tmpfiles";

  inputs = {
    nixpkgs.url = "github:rhelmot/nixpkgs/freebsd-staging";
    utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, utils }:
    let supportedSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-freebsd" ];
    in (utils.lib.eachSystem supportedSystems (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ self.overlays.default ];
        };
      in rec {
        packages.mini-tmpfiles = pkgs.mini-tmpfiles;
        packages.default = packages.mini-tmpfiles;

        apps.mini-tmpfiles = utils.lib.mkApp { drv = packages.mini-tmpfiles; };
        apps.default = apps.mini-tmpfiles;

        devShells.mini-tmpfiles = with pkgs;
          mkShell {
            packages = [
              rustPackages.cargo
              rustPackages.rustc
              rustPackages.rustfmt
              rustPackages.clippy
            ];
            RUST_SRC_PATH = "${rustPackages.rustPlatform.rustLibSrc}";
          };
        devShells.default = devShells.mini-tmpfiles;

        formatter = pkgs.nixfmt;
      })) // {
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
              platforms = supportedSystems;
            };
          };
        };
      };
}
