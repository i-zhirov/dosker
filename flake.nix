{
  description = "DOSker";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forEachSystem = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          dosker = pkgs.rustPlatform.buildRustPackage {
            pname = "dosker";
            version = "0.1.3";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "-p"
              "dosker-cli"
              "--bin"
              "dosker"
              "--features"
              "qemu-engine"
            ];
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postInstall = ''
              wrapProgram $out/bin/dosker \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.qemu ]}
            '';
            doCheck = false;
            meta = {
              description = "Docker-like system for pre-configured DOS systems";
              homepage = "https://github.com/i-zhirov/dosker";
              license = with pkgs.lib.licenses; [
                mit
                asl20
              ];
              mainProgram = "dosker";
            };
          };
        in
        {
          default = dosker;
          dosker = dosker;
        }
      );

      apps = forEachSystem (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/dosker";
        };
      });

      devShells = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages =
              with pkgs;
              [
                cargo
                clippy
                rustc
                rustfmt
                qemu
                dosbox-x
                mtools
                curl
                unzip
                jq
              ]
              ++ lib.optionals stdenv.hostPlatform.isLinux [
                bochs
                util-linux
              ];
          };
        }
      );

      checks = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          dosker = self.packages.${system}.default;
        in
        {
          default = dosker;
          version = pkgs.runCommand "dosker-version-check" { } ''
            test "$(${dosker}/bin/dosker --version)" = "dosker ${dosker.version}"
            touch $out
          '';
        }
      );
    };
}
