# Main derivation file for dns-smart-block workspace
{ pkgs
, lib
, crane
, rust-overlay
, system
}:

let
  # Set up Rust toolchain
  rustToolchain = pkgs.rust-bin.stable.latest.default.override {
    extensions = [ "rust-src" ];
  };

  # Set up crane library with our Rust toolchain
  craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

  # Common source filtering - exclude non-Rust files
  src = lib.cleanSourceWith {
    src = craneLib.path ../.;
    filter = path: type:
      # Keep all Rust source files, Cargo files, and README
      (lib.hasSuffix "\.rs" path) ||
      (lib.hasSuffix "\.toml" path) ||
      (lib.hasSuffix "\.lock" path) ||
      (lib.hasInfix "/src" path) ||
      (lib.hasInfix "/tests" path) ||
      (lib.hasInfix "/migrations" path) || # For SQLx migration files
      (lib.hasSuffix "\.txt" path) || # For prompt templates
      (craneLib.filterCargoSources path type);
  };

  # Common build inputs needed by all packages
  commonBuildInputs = [
    pkgs.pkg-config
    pkgs.openssl
  ];

  commonNativeBuildInputs = [
    pkgs.pkg-config
  ];

  # Build dependencies only (for caching)
  cargoArtifacts = craneLib.buildDepsOnly {
    inherit src;
    pname = "dns-smart-block-deps";
    buildInputs = commonBuildInputs;
    nativeBuildInputs = commonNativeBuildInputs;
  };

  # Common args for all builds
  commonArgs = {
    inherit src cargoArtifacts;
    buildInputs = commonBuildInputs;
    nativeBuildInputs = commonNativeBuildInputs;

    # Disable tests in Nix builds. All tests are integration tests that require
    # network connectivity (HTTP clients, mock servers, etc.) which is not
    # available in Nix's sandboxed build environment.
    doCheck = false;

    # Enable optimizations
    CARGO_PROFILE_RELEASE_LTO = "thin";
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "1";
  };

in
{
  # Individual package derivations
  classifier = craneLib.buildPackage (commonArgs // {
    pname = "dns-smart-block-classifier";
    version = "0.1.0";
    cargoExtraArgs = "--package dns-smart-block-classifier";

    # Install bundled prompt templates
    postInstall = ''
      mkdir -p $out/share/dns-smart-block/prompts
      cp ${../prompts}/*.txt $out/share/dns-smart-block/prompts/
    '';

    meta = {
      description = "DNS Smart Block Classifier - Fetches and classifies domains using LLM";
      homepage = "https://github.com/yourusername/dns-smart-block";
      license = lib.licenses.mit;
      maintainers = [ ];
    };
  });

  log-processor = craneLib.buildPackage (commonArgs // {
    pname = "dns-smart-block-log-processor";
    version = "0.1.0";
    cargoExtraArgs = "--package dns-smart-block-log-processor";

    meta = {
      description = "DNS Smart Block Log Processor - Watches DNS logs and queues domains";
      homepage = "https://github.com/yourusername/dns-smart-block";
      license = lib.licenses.mit;
      maintainers = [ ];
    };
  });

  queue-processor = craneLib.buildPackage (commonArgs // {
    pname = "dns-smart-block-queue-processor";
    version = "0.1.0";
    cargoExtraArgs = "--package dns-smart-block-queue-processor";

    meta = {
      description = "DNS Smart Block Queue Processor - Processes domains from NATS queue";
      homepage = "https://github.com/yourusername/dns-smart-block";
      license = lib.licenses.mit;
      maintainers = [ ];
    };
  });

  blocklist-server = craneLib.buildPackage (commonArgs // {
    pname = "dns-smart-block-blocklist-server";
    version = "0.1.0";
    cargoExtraArgs = "--package dns-smart-block-blocklist-server";

    meta = {
      description = "DNS Smart Block Blocklist Server - HTTP API for serving DNS blocklists";
      homepage = "https://github.com/yourusername/dns-smart-block";
      license = lib.licenses.mit;
      maintainers = [ ];
    };
  });

  # Combine all packages
  all = pkgs.symlinkJoin {
    name = "dns-smart-block-all";
    paths = [
      (lib.getExe' craneLib.buildPackage (commonArgs // {
        pname = "dns-smart-block-classifier";
        cargoExtraArgs = "--package dns-smart-block-classifier";
        postInstall = ''
          mkdir -p $out/share/dns-smart-block/prompts
          cp ${../prompts}/*.txt $out/share/dns-smart-block/prompts/
        '';
      }) "dns-smart-block-classifier")
      (lib.getExe' craneLib.buildPackage (commonArgs // {
        pname = "dns-smart-block-log-processor";
        cargoExtraArgs = "--package dns-smart-block-log-processor";
      }) "dns-smart-block-log-processor")
      (lib.getExe' craneLib.buildPackage (commonArgs // {
        pname = "dns-smart-block-queue-processor";
        cargoExtraArgs = "--package dns-smart-block-queue-processor";
      }) "dns-smart-block-queue-processor")
      (lib.getExe' craneLib.buildPackage (commonArgs // {
        pname = "dns-smart-block-blocklist-server";
        cargoExtraArgs = "--package dns-smart-block-blocklist-server";
      }) "dns-smart-block-blocklist-server")
    ];
  };
}
