{ pkgs, ... }:
let
  test-config = pkgs.writeText "test.conf" ''
    f /root/cat/girl 0777 root root - meow!
  '';
  pristine-file = pkgs.writeText "test.pristine" (builtins.readFile ./test.pristine);
in
pkgs.testers.runNixOSTest {
  name = "mini-test";
  nodes.machine =
    { config, pkgs, ... }:
    {
      boot.loader.systemd-boot.enable = true;
      environment.systemPackages = [
        pkgs.mini-tmpfiles
      ];
      # Once mini-tmpfiles is supports everything used during booting we can replace systemd-tmpfiles with it
      # systemd.services.systemd-tmpfiles-setup.serviceConfig = {
      #   ExecStart = [
      #     ""
      #     "${pkgs.mini-tmpfiles}/bin/mini-tmpfiles --create --remove --boot --exclude-prefix=/dev --exclude-prefix=/sysroot"
      #   ];
      # };
    };
  testScript = ''
    machine.wait_for_unit("default.target")
    machine.succeed("mini-tmpfiles --create ${test-config}")
    machine.succeed("{ ls -FloR --time-style=+\"\" /root; lsattr -R /root; } | diff ${pristine-file} -")
  '';
}
