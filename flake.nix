{
  description = "Standalone replacement for systemd-tmpfiles";

  inputs = {
    nixpkgs-freebsd.url = "github:rhelmot/nixpkgs/freebsd-staging";
    nixpkgs-nixos.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs-freebsd, nixpkgs-nixos, utils }:
    let supportedSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-freebsd" ];
    in (utils.lib.eachSystem supportedSystems (system:
      let
        nixpkgs = if system == "x86_64-freebsd" then nixpkgs-freebsd else nixpkgs-nixos;
        pkgs = import nixpkgs { inherit system; };
        inherit (pkgs) lib;
      in rec {
        packages.mini-tmpfiles = with pkgs;
          rustPlatform.buildRustPackage {
            name = "mini-tmpfiles";
            version = "0.1";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            doCheck = false;

            meta = with lib; {
              homepage = "https://github.com/nixos-bsd/mini-tmpfiles";
              description = "Standalone replacement for systemd-tmpfiles";
              maintainers = with maintainers; [ artemist ];
              license = with licenses; [ mit ];
              platforms = supportedSystems;
            };
          };
        defaultPackage = packages.mini-tmpfiles;

        apps.mini-tmpfiles = utils.lib.mkApp { drv = packages.rustybar; };
        defaultApp = apps.mini-tmpfiles;

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
        devShell = devShells.mini-tmpfiles;

        formatter = pkgs.nixfmt;
      })) // {
        overlays.default = final: prev: {
          inherit (self.packages.${prev.system}) mini-tmpfiles;
        };
      };
}
