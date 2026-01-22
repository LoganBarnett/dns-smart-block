{ config, lib, pkgs, ... }:

let
  cfg = config.services.dns-smart-block;
  inherit (lib) mkEnableOption mkOption mkIf types;

in {
  options.services.dns-smart-block = {
    enable = mkEnableOption "DNS Smart Block - LLM-powered DNS blocking";

    # NATS Configuration
    nats = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Enable built-in NATS server for dns-smart-block (recommended)";
      };

      port = mkOption {
        type = types.port;
        default = 4222;
        description = "NATS server port (change if you have another NATS instance)";
      };

      url = mkOption {
        type = types.str;
        default = "nats://localhost:${toString cfg.nats.port}";
        description = "NATS server URL (override if using external NATS)";
      };

      subject = mkOption {
        type = types.str;
        default = "dns.smart-block.domains";
        description = "NATS subject for domain messages";
      };

      dataDir = mkOption {
        type = types.path;
        default = "/var/lib/dns-smart-block-nats";
        description = "NATS data directory";
      };
    };

    # Log Processor Configuration
    logProcessor = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Enable log processor to watch DNS logs";
      };

      logSource = mkOption {
        type = types.str;
        default = "cmd:journalctl -f -u dnsdist";
        description = ''
          Log source to watch. Can be:
          - A file path: /var/log/dnsdist.log
          - A command: cmd:journalctl -f -u dnsdist
        '';
      };

      skipDnsdistCheck = mkOption {
        type = types.bool;
        default = false;
        description = "Skip checking dnsdist for already-blocked domains";
      };
    };

    # Queue Processor Configuration
    queueProcessor = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Enable queue processor to classify domains";
      };

      httpTimeoutSec = mkOption {
        type = types.int;
        default = 10;
        description = "HTTP timeout for fetching domains (seconds)";
      };

      httpMaxKb = mkOption {
        type = types.int;
        default = 100;
        description = "Maximum KB to download from each domain";
      };

      minConfidence = mkOption {
        type = types.float;
        default = 0.8;
        description = "Minimum confidence threshold to block (0.0 to 1.0)";
      };
    };

    # Ollama Configuration
    ollama = {
      url = mkOption {
        type = types.str;
        default = "http://localhost:11434";
        description = "Ollama server URL";
      };

      model = mkOption {
        type = types.str;
        default = "llama2";
        description = "Ollama model to use for classification";
      };
    };

    # dnsdist Configuration
    dnsdist = {
      apiUrl = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "http://localhost:8080";
        description = "dnsdist API URL for checking/blocking domains";
      };

      apiKeyFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = "/run/secrets/dnsdist-api-key";
        description = "Path to file containing dnsdist API key";
      };
    };

    # Classifier Configuration
    classifier = {
      preset = mkOption {
        type = types.nullOr (types.enum [ "gaming" ]);
        default = null;
        example = "gaming";
        description = ''
          Use a bundled classifier preset. Available presets:
          - "gaming" - Classifies gaming-related websites

          If set, the bundled prompt template will be used.
          If null, you must provide a custom promptTemplate.
        '';
      };

      customTemplate = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = "/etc/dns-smart-block/prompt-template.txt";
        description = ''
          Path to a custom LLM prompt template file.
          This file should contain {{INPUT_JSON}} placeholder.

          Only used if classifier.preset is null.
        '';
      };

      type = mkOption {
        type = types.str;
        default = "gaming";
        description = "Classification type label for database storage";
      };

      ttlDays = mkOption {
        type = types.int;
        default = 10;
        description = "Number of days classifications remain valid";
      };
    };

    # Database Configuration
    database = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Enable built-in PostgreSQL database (recommended)";
      };

      name = mkOption {
        type = types.str;
        default = "dns_smart_block";
        description = "PostgreSQL database name";
      };

      user = mkOption {
        type = types.str;
        default = "dns_smart_block";
        description = "PostgreSQL user name";
      };

      passwordFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = "/run/secrets/db-password";
        description = "Path to file containing database password (optional for local peer auth)";
      };

      host = mkOption {
        type = types.str;
        default = "/run/postgresql";
        description = "PostgreSQL host (use socket path for local peer auth)";
      };

      port = mkOption {
        type = types.port;
        default = 5432;
        description = "PostgreSQL port";
      };
    };

    # Package Options
    package = mkOption {
      type = types.package;
      default = pkgs.dns-smart-block-classifier;
      description = "Package providing dns-smart-block executables";
    };
  };

  config = mkIf cfg.enable (let
    # Determine which prompt template to use
    promptTemplate =
      if cfg.classifier.preset == "gaming" then
        "${cfg.package}/share/dns-smart-block/prompts/gaming-classifier.txt"
      else
        cfg.classifier.customTemplate;

    # Construct database URL
    databaseUrl =
      if cfg.database.host == "/run/postgresql" then
        # Unix socket connection with peer authentication
        "postgresql://${cfg.database.user}@/${cfg.database.name}?host=/run/postgresql"
      else
        # TCP connection
        "postgresql://${cfg.database.user}@${cfg.database.host}:${toString cfg.database.port}/${cfg.database.name}";
  in {

    # Install packages
    environment.systemPackages = [
      cfg.package
    ];

    # PostgreSQL Service
    services.postgresql = mkIf cfg.database.enable {
      enable = true;
      ensureDatabases = [ cfg.database.name ];
      ensureUsers = [{
        name = cfg.database.user;
        ensureDBOwnership = true;
      }];
    };

    # NATS Server Service
    systemd.services.dns-smart-block-nats = mkIf cfg.nats.enable {
      description = "NATS server for DNS Smart Block";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        Type = "simple";
        DynamicUser = true;
        StateDirectory = "dns-smart-block-nats";
        ExecStart = "${pkgs.nats-server}/bin/nats-server -p ${toString cfg.nats.port} -sd ${cfg.nats.dataDir}";
        Restart = "always";
        RestartSec = "5s";

        # Security hardening
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ cfg.nats.dataDir ];
      };
    };

    # Log Processor Service
    systemd.services.dns-smart-block-log-processor = mkIf cfg.logProcessor.enable {
      description = "DNS Smart Block Log Processor";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ]
        ++ lib.optional cfg.nats.enable "dns-smart-block-nats.service"
        ++ lib.optional cfg.database.enable "postgresql.service"
        ++ lib.optional (lib.hasPrefix "cmd:journalctl" cfg.logProcessor.logSource) "systemd-journald.service";
      wants = lib.optional cfg.nats.enable "dns-smart-block-nats.service"
        ++ lib.optional cfg.database.enable "postgresql.service";
      requires = lib.optional cfg.database.enable "postgresql.service";

      serviceConfig = {
        Type = "simple";
        DynamicUser = true;

        # Grant access to systemd journal if using journalctl and postgres for database
        SupplementaryGroups =
          lib.optional (lib.hasPrefix "cmd:journalctl" cfg.logProcessor.logSource) "systemd-journal"
          ++ lib.optional cfg.database.enable "postgres";

        ExecStart = let
          args = lib.concatStringsSep " " ([
            "${cfg.package}/bin/dns-smart-block-log-processor"
            "--log-source '${cfg.logProcessor.logSource}'"
            "--nats-url '${cfg.nats.url}'"
            "--nats-subject '${cfg.nats.subject}'"
            "--database-url '${databaseUrl}'"
          ] ++ lib.optionals (cfg.database.passwordFile != null) [
            "--database-password-file '${cfg.database.passwordFile}'"
          ] ++ lib.optionals (cfg.dnsdist.apiUrl != null) [
            "--dnsdist-api-url '${cfg.dnsdist.apiUrl}'"
          ] ++ lib.optionals cfg.logProcessor.skipDnsdistCheck [
            "--skip-dnsdist-check"
          ]);
        in args;

        Restart = "always";
        RestartSec = "5s";

        # Security hardening
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;

        # Read-only access to logs if using file source
        ReadOnlyPaths = lib.optional (!(lib.hasPrefix "cmd:" cfg.logProcessor.logSource)) cfg.logProcessor.logSource;
      };

      environment = {
        RUST_LOG = "info";
      } // lib.optionalAttrs (cfg.dnsdist.apiKeyFile != null) {
        DNSDIST_API_KEY = "$(cat ${cfg.dnsdist.apiKeyFile})";
      };
    };

    # Queue Processor Service
    systemd.services.dns-smart-block-queue-processor = mkIf cfg.queueProcessor.enable {
      description = "DNS Smart Block Queue Processor";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ]
        ++ lib.optional cfg.nats.enable "dns-smart-block-nats.service"
        ++ lib.optional cfg.database.enable "postgresql.service";
      wants = lib.optional cfg.nats.enable "dns-smart-block-nats.service"
        ++ lib.optional cfg.database.enable "postgresql.service";
      requires = lib.optional cfg.database.enable "postgresql.service";

      serviceConfig = {
        Type = "simple";
        DynamicUser = true;

        # Grant postgres group membership for peer auth
        SupplementaryGroups = lib.optional cfg.database.enable "postgres";

        ExecStart = let
          args = lib.concatStringsSep " " ([
            "${cfg.package}/bin/dns-smart-block-queue-processor"
            "--nats-url '${cfg.nats.url}'"
            "--nats-subject '${cfg.nats.subject}'"
            "--database-url '${databaseUrl}'"
            "--classifier-path '${cfg.package}/bin/dns-smart-block-classifier'"
            "--ollama-url '${cfg.ollama.url}'"
            "--ollama-model '${cfg.ollama.model}'"
            "--prompt-template '${promptTemplate}'"
            "--classification-type '${cfg.classifier.type}'"
            "--http-timeout-sec ${toString cfg.queueProcessor.httpTimeoutSec}"
            "--http-max-kb ${toString cfg.queueProcessor.httpMaxKb}"
            "--min-confidence ${toString cfg.queueProcessor.minConfidence}"
            "--classification-ttl-days ${toString cfg.classifier.ttlDays}"
          ] ++ lib.optionals (cfg.database.passwordFile != null) [
            "--database-password-file '${cfg.database.passwordFile}'"
          ] ++ lib.optionals (cfg.dnsdist.apiUrl != null) [
            "--dnsdist-api-url '${cfg.dnsdist.apiUrl}'"
          ]);
        in args;

        Restart = "always";
        RestartSec = "5s";

        # Security hardening
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;

        # Read access to prompt template
        ReadOnlyPaths = [ promptTemplate ];
      };

      environment = {
        RUST_LOG = "info";
      } // lib.optionalAttrs (cfg.dnsdist.apiKeyFile != null) {
        DNSDIST_API_KEY = "$(cat ${cfg.dnsdist.apiKeyFile})";
      };
    };

    # Assertions and warnings
    assertions = [
      {
        assertion = cfg.queueProcessor.enable -> (cfg.classifier.preset != null || cfg.classifier.customTemplate != null);
        message = "services.dns-smart-block: Either classifier.preset or classifier.customTemplate must be set when queue processor is enabled";
      }
      {
        assertion = cfg.logProcessor.enable -> (cfg.nats.enable || cfg.nats.url != "");
        message = "NATS must be enabled or URL configured when log processor is enabled";
      }
      {
        assertion = cfg.queueProcessor.minConfidence >= 0.0 && cfg.queueProcessor.minConfidence <= 1.0;
        message = "services.dns-smart-block.queueProcessor.minConfidence must be between 0.0 and 1.0";
      }
    ];

    warnings =
      lib.optional (!cfg.nats.enable && cfg.nats.url == "nats://localhost:${toString cfg.nats.port}")
        "DNS Smart Block: Built-in NATS is disabled but no external NATS URL configured"
      ++
      lib.optional (cfg.dnsdist.apiUrl == null)
        "DNS Smart Block: No dnsdist API URL configured - blocking functionality will be limited";
  });
}
