{
  pkgs,
  reference ? false,
  ...
}:
let
  test-config = pkgs.writeText "test.conf" ''
    f /root/cat/girl 0777 root root - meow!
    d /root/cat 0500
    L+ /root/booted-system 0755 root root - /run/booted-system
    L+ /root/current-system - - - - /run/current-system
    f /root/clobbered
    f= /root/clobbered/clobbering
  '';
  print-script = pkgs.writeText "print-script.sh" ''
    ls -FloRA --time-style=+"" /root
    find /root -print0 | sort --zero-terminated | xargs --null lsattr -d || true
  '';
  pristine-file = pkgs.writeText "test.pristine" (builtins.readFile ./test.pristine);
  tmpfiles = if reference then "systemd-tmpfiles" else "mini-tmpfiles";
in
pkgs.testers.runNixOSTest {
  name = if reference then "reference-test" else "mini-test";
  nodes.machine =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    {
      boot.loader.systemd-boot.enable = true;
      environment.systemPackages = lib.optionals (!reference) [
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
    machine.succeed("${tmpfiles} --create ${test-config}")
    machine.succeed("bash ${print-script} | diff ${pristine-file} -")
  '';
}
