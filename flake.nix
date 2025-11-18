{
  description = "A voice operated computer like this is the future.";
  inputs = {
    nixpkgs.url = github:NixOS/nixpkgs/nixpkgs-unstable;
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, rust-overlay, crane }@inputs: let
    systems = [
      "aarch64-darwin"
      "aarch64-linux"
      "x86_64-darwin"
      "x86_64-linux"
    ];
    forAllSystems = f: nixpkgs.lib.genAttrs systems f;
    overlays = [
      (import rust-overlay)
    ];
    pkgsFor = system: import nixpkgs { inherit overlays system; };
    packages = (pkgs: let
      rust = pkgs.rust-bin.stable.latest.default.override {
        extensions = [
          # For rust-analyzer and others.  See
          # https://nixos.wiki/wiki/Rust#Shell.nix_example for some details.
          "rust-src"
          "rust-analyzer"
          "rustfmt"
        ];
      };
    in [
      pkgs.cargo-sweep
      pkgs.clang
      # To help build whisper-rs from whisper-cpp.
      pkgs.cmake
      # To help with finding openssl.
      pkgs.pkg-config
      rust
    ]);
  in {

    devShells = forAllSystems (system: {
      default = (pkgsFor system).mkShell {
        buildInputs = (packages (pkgsFor system));
        shellHook = ''
        '';
      };
    });

    nixosModules.default = ./nix/nixos-module.nix;

    overlays.default = final: prev: {
      github-to-jenkins-webhook = final.callPackage ./nix/derivation.nix {
        inherit crane;
      };
    };

    packages = forAllSystems (system: {
      default = (pkgsFor system).callPackage ./nix/derivation.nix {
        inherit crane;
      };
    });

  };

}
