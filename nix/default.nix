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
      (lib.hasSuffix "\.txt" path) || # For prompt templates
      (craneLib.filterCargoSources path type);
  };

  # Common build inputs needed by all packages
  commonBuildInputs = with pkgs; [
    pkg-config
    openssl
  ];

  commonNativeBuildInputs = with pkgs; [
    pkg-config
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

    # Enable optimizations
    CARGO_PROFILE_RELEASE_LTO = "thin";
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "1";
  };

in
{
  # Individual package derivations
  worker = craneLib.buildPackage (commonArgs // {
    pname = "dns-smart-block-worker";
    version = "0.1.0";
    cargoExtraArgs = "--package dns-smart-block-worker";

    # Install bundled prompt templates
    postInstall = ''
      mkdir -p $out/share/dns-smart-block/prompts
      cp ${../prompts}/*.txt $out/share/dns-smart-block/prompts/
    '';

    meta = with lib; {
      description = "DNS Smart Block Worker - Fetches and classifies domains using LLM";
      homepage = "https://github.com/yourusername/dns-smart-block";
      license = licenses.mit;
      maintainers = [ ];
    };
  });

  log-processor = craneLib.buildPackage (commonArgs // {
    pname = "dns-smart-block-log-processor";
    version = "0.1.0";
    cargoExtraArgs = "--package dns-smart-block-log-processor";

    meta = with lib; {
      description = "DNS Smart Block Log Processor - Watches DNS logs and queues domains";
      homepage = "https://github.com/yourusername/dns-smart-block";
      license = licenses.mit;
      maintainers = [ ];
    };
  });

  queue-processor = craneLib.buildPackage (commonArgs // {
    pname = "dns-smart-block-queue-processor";
    version = "0.1.0";
    cargoExtraArgs = "--package dns-smart-block-queue-processor";

    meta = with lib; {
      description = "DNS Smart Block Queue Processor - Processes domains from NATS queue";
      homepage = "https://github.com/yourusername/dns-smart-block";
      license = licenses.mit;
      maintainers = [ ];
    };
  });

  # Combine all packages
  all = pkgs.symlinkJoin {
    name = "dns-smart-block-all";
    paths = [
      (lib.getExe' craneLib.buildPackage (commonArgs // {
        pname = "dns-smart-block-worker";
        cargoExtraArgs = "--package dns-smart-block-worker";
      }) "dns-smart-block-worker")
      (lib.getExe' craneLib.buildPackage (commonArgs // {
        pname = "dns-smart-block-log-processor";
        cargoExtraArgs = "--package dns-smart-block-log-processor";
      }) "dns-smart-block-log-processor")
      (lib.getExe' craneLib.buildPackage (commonArgs // {
        pname = "dns-smart-block-queue-processor";
        cargoExtraArgs = "--package dns-smart-block-queue-processor";
      }) "dns-smart-block-queue-processor")
    ];
  };
}
