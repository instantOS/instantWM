{
  description = "instantWM - hybrid tiling window manager for X11 and Wayland";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
  }: let
    systems = ["x86_64-linux"];
    forAllSystems = nixpkgs.lib.genAttrs systems;

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

    devShells = forAllSystems (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
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
        };
      }
    );

    checks = forAllSystems (system: {
      inherit (self.packages.${system}) default;
    });
  };
}
