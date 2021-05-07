let
  str_sleep_time = builtins.toString 1;
  # For extra determinism
  nixpkgs = builtins.fetchTarball {
    url = "http://github.com/NixOS/nixpkgs/archive/389249fa9b35b3071b4ccf71a3c065e7791934df.tar.gz";
    sha256 = "1z087f1m1k4pii2v2xai8n0yd3m57svgslzzbm7fwdjjzhn8g2rl";
  };
  shamilton = import (builtins.fetchTarball {
    url = "https://github.com/SCOTT-HAMILTON/nur-packages/tarball/9bd7ba3";
    sha256 = "1mimljrgffmhm0hv60h9bjiiwhb069m7g1fxnss4nfr5vz1yjady";
  }) {};
  instantnix = import (builtins.fetchTarball {
    url = "https://github.com/instantOS/instantNIX/tarball/b27d2b6";
    sha256 = "0x0yv408680qafxfg0na8428sl4q8z43gv5lb11rsii2lkj87rx8";
  }) {};
  pkgs = import nixpkgs {};
  instrumented-instantwm = with pkgs; callPackage ../. {
    buildInstrumentedCoverage = true;
    inherit (nix-gitignore) gitignoreSource;
    inherit (instantnix)
      instantassist
      instantutils
      instantdotfiles;
  };
  source = ../.;

  runInstantwm = pkgs.writeScriptBin "run-instantwm" ''
    #!${pkgs.stdenv.shell}
    export LLVM_PROFILE_FILE='instantwm-%p.profraw'
    Xephyr -ac -screen 1366x720 -br -reset -terminate :1 &
    sleep 3
    export DISPLAY=:1.0
    timeout 10m ${instrumented-instantwm}/bin/instantwm &
  '';
in
  import "${nixpkgs}/nixos/tests/make-test-python.nix" ({ pkgs, ...}: {
    system = "x86_64-linux";

    nodes.machine = { nodes, config, pkgs, ... }:
    {
      imports = [
        "${nixpkgs}/nixos/tests/common/user-account.nix"
        "${nixpkgs}/nixos/tests/common/x11.nix"
      ];
      environment.systemPackages = with pkgs; [
        binutils
        coreutils
        glibc
        gnugrep
        gnused
        instrumented-instantwm
        llvmPackages_11.bintools
        runInstantwm
        xdotool
        killall
      ] ++ [
        instantnix.instantconf
        instantnix.instantdata
        instantnix.instantmenu
        instantnix.instantutils
        instantnix.instantwallpaper
        instantnix.instantwelcome
        libnotify
        xdg-user-dirs
      ];
    };

    enableOCR = true;

    testScript = ''
      import os

      start_all()

      sleep_time = int(${str_sleep_time})

      # Copy sources to instantwm directory
      machine.succeed("cp -r ${source} instantwm")
      machine.wait_for_x()

      # machine.wait_for_text("root@machine")
      machine.succeed("run-instantwm")
      machine.sleep(sleep_time * 5)
      machine.screenshot("screen1")
      
      ### Normal Use case sequences
      machine.send_key("ctrl-shift-ret")
      machine.sleep(sleep_time * 3)
      machine.send_key("alt-f4")
      machine.sleep(sleep_time * 3)
      machine.screenshot("screen2")

      ### Cleanup
      machine.succeed("killall .instantautostart-wrapped")
      machine.succeed("killall .instantstatus-wrapped")
      machine.succeed("killall Xephyr")

      machine.succeed(
          "llvm-profdata merge -sparse *.profraw -o instantwm.profdata",
          "llvm-cov export -ignore-filename-regex='.*config.h.*' ${instrumented-instantwm}/bin/instantwm -format=lcov -instr-profile=instantwm.profdata > instantwm.lcov",
          "sed -i 's=/build/instantWM/==g' instantwm.lcov",
      )
      machine.copy_from_vm("instantwm.lcov", "coverage_data")
      machine.copy_from_vm("instantwm.profdata", "coverage_data")
      out_dir = os.environ.get("out", os.getcwd())
      eprint('Coverage data written to "{}/coverage_data/instantwm.lcov"'.format(out_dir))
      machine.screenshot("screen3")
    '';
})
