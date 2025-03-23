{
  pkgs,
  reference ? false,
}@args:

let
  before-script = pkgs.writeShellScript "before-script.sh" ''
    mkdir /root/existing-dir
  '';
  test-config = pkgs.writeText "test.conf" ''
    f /root/cat/girl 0777 root root - meow!
    d /root/cat 0500
    L+ /root/booted-system 0755 root root - /run/booted-system
    L+ /root/current-system - - - - /run/current-system
    f /root/clobbered
    f= /root/clobbered/clobbering
    z /root/nonexistent 0000
    z /root/existing-dir 0000
  '';
  print-script = pkgs.writeShellScript "print-script.sh" ''
    ls -FloRA --time-style=+"" /root
    find /root -print0 | sort --zero-terminated | xargs --null lsattr -d || true
  '';
  testFile = (
    {
      generating ? false,
    }:
    let
      generate-output = ''
        machine.wait_for_unit("default.target")
        machine.succeed("${before-script}")
        machine.succeed("${tmpfiles} --create ${test-config}")
        machine.succeed("bash ${print-script} > output")
      '';
      tmpfiles = if reference || generating then "systemd-tmpfiles" else "mini-tmpfiles";
      generate-testscript = pkgs.writeText "generate-testscript" (
        generate-output
        + ''
          machine.copy_from_vm(source="output")
        ''
      );
      generated = (testFile { generating = true; }).generate;
      test = pkgs.testers.runNixOSTest {
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
            environment.systemPackages = lib.optionals (!(reference || generating)) [
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
        testScript = pkgs.lib.optionalString (!generating) (
          generate-output
          + ''
            machine.succeed("diff ${generated} output")
          ''
        );
        passthru.generate =
          if generating then
            pkgs.runCommand "generated" { } ''
              ${test.driver}/bin/nixos-test-driver ${generate-testscript}
              mv output $out
            ''
          else
            generated;
      };
    in
    test
  );
in
testFile { }
