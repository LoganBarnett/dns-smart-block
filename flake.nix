{
  description = "DNS Smart Block - LLM-powered intelligent DNS blocking";
  inputs = {
    nixpkgs.url = github:NixOS/nixpkgs/25.11;
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
    pkgsFor = system: import nixpkgs {
      inherit system;
      overlays = overlays;
    };

    # Development shell packages.
    devPackages = pkgs: let
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
      rust
      pkgs.cargo-sweep
      pkgs.pkg-config
      pkgs.openssl
    ];
  in {

    devShells = forAllSystems (system: {
      default = (pkgsFor system).mkShell {
        buildInputs = devPackages (pkgsFor system);
        shellHook = ''
          echo "DNS Smart Block development environment"
          echo "Available packages: classifier, log-processor, queue-processor, blocklist-server"
        '';
      };
    });

    packages = forAllSystems (system: let
      pkgs = pkgsFor system;
      dnsSmartBlock = pkgs.callPackage ./nix/default.nix {
        inherit crane;
        inherit (inputs) rust-overlay;
        inherit system;
      };
    in {
      # Individual packages
      classifier = dnsSmartBlock.classifier;
      log-processor = dnsSmartBlock.log-processor;
      queue-processor = dnsSmartBlock.queue-processor;
      blocklist-server = dnsSmartBlock.blocklist-server;

      # Default to building all
      default = dnsSmartBlock.all;
    });

    overlays.default = final: prev: {
      dns-smart-block-classifier = self.packages.${final.system}.classifier;
      dns-smart-block-log-processor = self.packages.${final.system}.log-processor;
      dns-smart-block-queue-processor = self.packages.${final.system}.queue-processor;
      dns-smart-block-blocklist-server = self.packages.${final.system}.blocklist-server;
    };

    # Apps for easy running
    apps = forAllSystems (system: {
      classifier = {
        type = "app";
        program = "${self.packages.${system}.classifier}/bin/dns-smart-block-classifier";
      };
      log-processor = {
        type = "app";
        program = "${self.packages.${system}.log-processor}/bin/dns-smart-block-log-processor";
      };
      queue-processor = {
        type = "app";
        program = "${self.packages.${system}.queue-processor}/bin/dns-smart-block-queue-processor";
      };
      blocklist-server = {
        type = "app";
        program = "${self.packages.${system}.blocklist-server}/bin/dns-smart-block-blocklist-server";
      };
    });

    # NixOS Module
    nixosModules.default = import ./nix/nixos-module.nix;

  };

}
