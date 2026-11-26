{
  description = "instantWM - hybrid tiling window manager for X11 and Wayland";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
  }: let
    systems = ["x86_64-linux" "aarch64-linux"];
    forAllSystems = nixpkgs.lib.genAttrs systems;

    # Full revision (not shortRev), because build.rs truncates the commit to
    # eight characters for IPC_PROTOCOL_VERSION; a seven-character shortRev
    # would not match a ctl built from a Git checkout. Drop the "-dirty"
    # suffix so the embedded commit matches `git rev-parse HEAD`.
    sourceRevision = nixpkgs.lib.removeSuffix "-dirty" (
      self.rev or self.dirtyRev or "unknown"
    );

    mkInstantwm = pkgs:
      pkgs.rustPlatform.buildRustPackage {
        pname = "instantwm";

        version = (pkgs.lib.importTOML ./Cargo.toml).package.version;

        src = ./.;

        cargoLock = {
          lockFile = ./Cargo.lock;

          outputHashes = {
            "instantos-ipc-0.1.0" = "sha256-Do2JIcbtOscqyevf4dh+ME088CdOARLbG5pItD5nzIU=";
            "instantos-terminal-0.1.0" = "sha256-Do2JIcbtOscqyevf4dh+ME088CdOARLbG5pItD5nzIU=";
            "smithay-0.7.0" = "sha256-Wr+wonmUFcM9LxNIsVhkLrAploVPLp1GJtlZzb3ugJk=";
          };
        };

        # Nix's source copy has no .git, so build.rs cannot run `git rev-parse`
        # itself. Pass the flake revision through explicitly to keep the IPC
        # protocol version and `instantwmctl status` commit in sync with builds
        # made from a Git checkout.
        env.INSTANTWM_BUILD_COMMIT = sourceRevision;

        nativeBuildInputs = [
          pkgs.pkg-config
          pkgs.cmake
          pkgs.patchelf
        ];

        buildInputs = with pkgs; [
          # Direct link (build.rs)
          libx11
          libxinerama
          libxft
          libxrender
          fontconfig
          freetype

          # X11 stack used through x11rb
          libxcb
          libxcursor
          libxrandr
          libxfixes
          libxdamage
          libxcomposite
          libxtst
          libxi
          libxres
          libxscrnsaver
          libxxf86vm
          libxvmc
          libxt
          libxmu
          libxpm
          libxaw

          # Wayland compositor stack (smithay)
          libxkbcommon
          wayland
          wayland-protocols
          libinput
          seatd
          libdrm
          mesa
          libglvnd
          libgbm
          libevdev
          libwacom
          udev
          dbus
        ];

        # instantwm-portals.conf routes Screenshot and ScreenCast to the wlr
        # backend. Keep it in the package closure for dev shells and
        # non-NixOS setups; nixosModules below wires up runtime discovery.
        propagatedBuildInputs = [pkgs.xdg-desktop-portal-wlr];

        doCheck = false;

        # wayland-client is built with `dlopen`, and smithay loads EGL the same
        # way: neither shows up in `ldd`, so the loader needs a hint or the
        # Wayland backend dies with `NoWaylandLib`. RPATH is used instead of a
        # wrapper so LD_LIBRARY_PATH does not leak into every app the WM starts.
        postFixup = ''
          for bin in instantwm instantwmctl; do
            patchelf --add-rpath ${
            pkgs.lib.makeLibraryPath [
              pkgs.wayland
              pkgs.libglvnd
              pkgs.libdrm
              pkgs.libgbm
              pkgs.mesa
            ]
          } $out/bin/$bin
          done
        '';

        postInstall = ''
          install -Dm755 scripts/startinstantos $out/bin/startinstantos
          install -Dm755 scripts/instantwm-debug.sh $out/bin/instantwm-debug
          install -Dm644 instantwm.1 $out/share/man/man1/instantwm.1
          install -Dm644 LICENSE $out/share/licenses/instantwm/LICENSE

          install -Dm644 utils/instantwm-x11.desktop \
            $out/share/xsessions/instantwm.desktop
          install -Dm644 utils/instantwm-wayland.desktop \
            $out/share/wayland-sessions/instantwm-wayland.desktop
          install -Dm644 utils/instantwm-wayland-debug.desktop \
            $out/share/wayland-sessions/instantwm-debug.desktop

          install -Dm644 resources/instantwm-portals.conf \
            $out/share/xdg-desktop-portal/instantwm-portals.conf
          install -Dm644 resources/instantwm.portal \
            $out/share/xdg-desktop-portal/portals/instantwm.portal
          install -Dm644 resources/instantwm-session.target \
            $out/lib/systemd/user/instantwm-session.target
        '';

        passthru.providedSessions = [
          "instantwm"
          "instantwm-wayland"
          "instantwm-debug"
        ];

        meta = {
          description = "Hybrid tiling window manager for X11 and Wayland";
          homepage = "https://github.com/instantOS/instantWM";
          license = pkgs.lib.licenses.mit;
          mainProgram = "instantwm";
        };
      };
  in {
    packages = forAllSystems (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
      in {
        default = mkInstantwm pkgs;
        instantwm = self.packages.${system}.default;
      }
    );

    overlays.default = final: _prev: {instantwm = mkInstantwm final;};

    nixosModules = rec {
      instantwm = {
        config,
        lib,
        pkgs,
        ...
      }: let
        cfg = config.programs.instantwm;
        package = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      in {
        options.programs.instantwm = {
          enable = lib.mkEnableOption "the instantWM window manager";

          package = lib.mkOption {
            type = lib.types.package;
            default = package;
            defaultText = lib.literalExpression "instantwm.packages.${pkgs.stdenv.hostPlatform.system}.default";
            description = "The instantWM package to install.";
          };
        };

        config = lib.mkIf cfg.enable {
          hardware.graphics.enable = lib.mkDefault true;
          programs.xwayland.enable = lib.mkDefault true;

          environment.systemPackages = [cfg.package];

          # Expose the X11 and Wayland sessions to display managers.
          services.displayManager.sessionPackages = [cfg.package];

          # Provides instantwm-session.target and any future user units.
          systemd.packages = [cfg.package];

          # instantwm-portals.conf routes Screenshot and ScreenCast to the
          # wlr backend and uses gtk as the default fallback.
          xdg.portal = {
            enable = lib.mkDefault true;
            extraPortals = [
              pkgs.xdg-desktop-portal-wlr
              pkgs.xdg-desktop-portal-gtk
            ];
            configPackages = [cfg.package];
          };
        };
      };

      default = instantwm;
    };

    devShells = forAllSystems (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
        nestedBackendLibs = pkgs.lib.makeLibraryPath [
          pkgs.libglvnd
          pkgs.mesa
          pkgs.libdrm
          pkgs.libgbm
          pkgs.wayland
        ];
      in {
        default = pkgs.mkShell {
          inputsFrom = [self.packages.${system}.default];
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rust-analyzer
            pkgs.just
            pkgs.uv
            pkgs.python3
          ];

          LD_LIBRARY_PATH = nestedBackendLibs;
        };
      }
    );

    checks = forAllSystems (system: {
      inherit (self.packages.${system}) default;
    });
  };
}
