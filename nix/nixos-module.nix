{ config, lib, pkgs, ... }: let
  cfg = config.services.dns-smart-block;
  inherit (lib) mkEnableOption mkOption mkIf types;

  classifierType = types.submodule {
    options = {
      enable = mkOption {
        type = types.bool;
        default = false;
        description = "Enable this classifier";
      };

      preset = mkOption {
        type = types.nullOr (types.enum [ "gaming" "video-streaming" "social-media" ]);
        default = null;
        example = "gaming";
        description = ''
          Use a bundled classifier preset. Available presets:
          - "gaming" - Classifies gaming-related websites
          - "video-streaming" - Classifies video streaming platforms
          - "social-media" - Classifies social media, chat, and messaging platforms

          If set, the bundled prompt template will be used.
          If null, you must provide a customTemplate.
        '';
      };

      customTemplate = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = "/etc/dns-smart-block/custom-prompt.txt";
        description = ''
          Path to a custom LLM prompt template file.
          This file should contain {{INPUT_JSON}} placeholder.

          Only used if preset is null.
        '';
      };

      httpTimeoutSec = mkOption {
        type = types.int;
        default = cfg.queueProcessor.httpTimeoutSec;
        description = "HTTP timeout for fetching domains (seconds)";
      };

      httpMaxKb = mkOption {
        type = types.int;
        default = cfg.queueProcessor.httpMaxKb;
        description = "Maximum KB to download from each domain";
      };

      minConfidence = mkOption {
        type = types.float;
        default = cfg.queueProcessor.minConfidence;
        description = "Minimum confidence threshold to block (0.0 to 1.0)";
      };

      ttlDays = mkOption {
        type = types.int;
        default = 30;
        description = "Time-to-live in days for cached classifications. After this many days, a domain will be re-classified.";
      };
    };
  };

in {
  imports = [
    ./integrations/blocky.nix
    ./prometheus-exporter.nix
  ];

  options.services.dns-smart-block = {
    enable = mkEnableOption "DNS Smart Block - LLM-powered DNS blocking";

    # Classifiers Configuration
    classifiers = mkOption {
      type = types.attrsOf classifierType;
      default = {};
      example = {
        gaming = {
          enable = true;
          preset = "gaming";
        };
        video-streaming = {
          enable = true;
          preset = "video-streaming";
          minConfidence = 0.85;
        };
      };
      description = ''
        Attribute set of classifiers to run. Each classifier runs as a separate
        queue processor service. The attribute name is used as the classification
        type in the database.

        Example configuration:
          classifiers.gaming.enable = true;
          classifiers.gaming.preset = "gaming";

          classifiers.video-streaming.enable = true;
          classifiers.video-streaming.preset = "video-streaming";
      '';
    };

    # NATS Configuration
    nats = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Enable built-in NATS server for dns-smart-block (recommended)";
      };

      url = mkOption {
        type = types.str;
        default = "nats://localhost:4222";
        description = "NATS server URL.  Override when pointing at an external NATS instance.";
      };

      subject = mkOption {
        type = types.str;
        default = "dns.smart-block.domains";
        description = "NATS subject for domain messages";
      };

      maxAckPending = mkOption {
        type = types.int;
        default = 1;
        description = ''
          Maximum number of unacknowledged messages allowed per consumer.
          Setting this to 1 ensures that each queue processor handles only one
          message at a time. With multiple processors running, the total
          concurrency will be (number of enabled classifiers × maxAckPending).
        '';
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
        default = "cmd:journalctl -f -u dns-server";
        description = ''
          Log source to watch. Can be:
          - A file path: /var/log/dns.log
          - A command: cmd:journalctl -f -u dns-server
        '';
      };
    };

    # Queue Processor Global Defaults
    queueProcessor = {
      httpTimeoutSec = mkOption {
        type = types.int;
        default = 120;
        description = "Default HTTP timeout for fetching domains (seconds)";
      };

      httpMaxKb = mkOption {
        type = types.int;
        default = 100;
        description = "Default maximum KB to download from each domain";
      };

      minConfidence = mkOption {
        type = types.float;
        default = 0.8;
        description = "Default minimum confidence threshold to block (0.0 to 1.0)";
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

      numCtx = mkOption {
        type = types.nullOr types.ints.positive;
        default = 4096;
        description = ''
          Context window size passed to Ollama.  Controls the KV cache
          allocation at model load time.  The default of 4096 provides ample
          headroom for classification prompts while avoiding the much larger
          model-native default that wastes GPU memory.  Set to null to omit
          the option and let Ollama use its own default.
        '';
      };
    };

    # Domain Exclusions
    excludeSuffixes = mkOption {
      type = types.listOf types.str;
      default = [];
      example = [ ".example.com" ".local" ];
      description = ''
        Domain suffixes excluded from LLM classification.  Domains whose names
        end with any listed suffix receive a synthetic "not matching"
        classification at confidence 1.0, creating an audit trail without
        invoking the LLM.

        Use leading-dot notation (e.g. <literal>.example.com</literal>) to match a
        TLD and all names under it.  A bare name (e.g.
        <literal>example.com</literal>) matches any domain whose string
        representation ends with it, including subdomains.
      '';
    };

    # Blocklist Server Configuration
    blocklistServer = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Enable HTTP blocklist server for serving DNS blocklists";
      };

      publicBindHost = mkOption {
        type = types.str;
        default = "0.0.0.0";
        description = "Host to bind the public server to (blocklist, metrics, health)";
      };

      publicBindPort = mkOption {
        type = types.port;
        default = 3000;
        description = "Port to bind the public server to (blocklist, metrics, health)";
      };

      adminBindHost = mkOption {
        type = types.str;
        default = "127.0.0.1";
        description = "Host to bind the admin server to (classifications, reprojection)";
      };

      adminBindPort = mkOption {
        type = types.port;
        default = 8080;
        description = "Port to bind the admin server to (classifications, reprojection)";
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
  };

  config = mkIf cfg.enable (let
    # Package references (internal, not meant for user configuration)
    packages = {
      classifier = pkgs.dns-smart-block-classifier;
      log-processor = pkgs.dns-smart-block-log-processor;
      queue-processor = pkgs.dns-smart-block-queue-processor;
      blocklist-server = pkgs.dns-smart-block-blocklist-server;
    };

    # Static user for services to enable PostgreSQL peer authentication.
    # This user is shared by all dns-smart-block services.
    serviceUser = "dns_smart_block";
    serviceGroup = "dns_smart_block";

    # Construct database URL.
    databaseUrl = let
      rawUrl =
        if cfg.database.host == "/run/postgresql" then
          # Unix socket connection with peer authentication. URL-encode the socket
          # path: %2F = /, %2Frun%2Fpostgresql = /run/postgresql
          "postgresql://${cfg.database.user}@%2Frun%2Fpostgresql/${cfg.database.name}"
        else
          # TCP connection.
          "postgresql://${cfg.database.user}@${cfg.database.host}:${toString cfg.database.port}/${cfg.database.name}";
    in
      # Systemd interprets % as a specifier, so escape it as %%.
      lib.replaceStrings ["%"] ["%%"] rawUrl;

    # Get list of enabled classifiers.
    enabledClassifiers = lib.filterAttrs (name: classifier: classifier.enable) cfg.classifiers;

    # Generate TOML configuration for the queue processor.
    # This creates a single config file with all enabled classifiers.
    queueProcessorTomlConfig = let
      # Generate classifier sections.
      classifierSections = lib.concatStringsSep "\n\n" (
        lib.mapAttrsToList (classifierName: classifier: let
          # Determine which prompt template to use.
          promptTemplate =
            if classifier.preset == "gaming" then
              "${packages.classifier}/share/dns-smart-block/prompts/gaming-classifier.txt"
            else if classifier.preset == "video-streaming" then
              "${packages.classifier}/share/dns-smart-block/prompts/video-streaming-classifier.txt"
            else if classifier.preset == "social-media" then
              "${packages.classifier}/share/dns-smart-block/prompts/social-media-classifier.txt"
            else
              classifier.customTemplate;
        in ''
          [[classifier]]
          name = "${classifierName}"
          prompt_template = "${promptTemplate}"
          ${lib.optionalString (classifier.minConfidence != cfg.queueProcessor.minConfidence)
            "min_confidence = ${toString classifier.minConfidence}"}
          ${lib.optionalString (classifier.ttlDays != 7)
            "ttl_days = ${toString classifier.ttlDays}"}
          ${lib.optionalString (classifier.httpTimeoutSec != cfg.queueProcessor.httpTimeoutSec)
            "http_timeout_sec = ${toString classifier.httpTimeoutSec}"}
          ${lib.optionalString (classifier.httpMaxKb != cfg.queueProcessor.httpMaxKb)
            "http_max_kb = ${toString classifier.httpMaxKb}"}
        '') enabledClassifiers
      );
    in pkgs.writeText "dns-smart-block-queue-processor.toml" ''
      # DNS Smart Block Queue Processor Configuration
      # Generated by NixOS module

      [ollama]
      url = "${cfg.ollama.url}"
      model = "${cfg.ollama.model}"
      ${lib.optionalString (cfg.ollama.numCtx != null) "num_ctx = ${toString cfg.ollama.numCtx}"}

      [http]
      timeout_sec = ${toString cfg.queueProcessor.httpTimeoutSec}
      max_kb = ${toString cfg.queueProcessor.httpMaxKb}

      [defaults]
      min_confidence = ${toString cfg.queueProcessor.minConfidence}
      ttl_days = 7

      ${lib.optionalString (cfg.excludeSuffixes != []) ''
      exclude_suffixes = ${builtins.toJSON cfg.excludeSuffixes}
      ''}

      ${classifierSections}
    '';

    # All prompt templates referenced in the config (for ReadOnlyPaths).
    allPromptTemplates = lib.flatten (
      lib.mapAttrsToList (classifierName: classifier:
        if classifier.preset == "gaming" then
          "${packages.classifier}/share/dns-smart-block/prompts/gaming-classifier.txt"
        else if classifier.preset == "video-streaming" then
          "${packages.classifier}/share/dns-smart-block/prompts/video-streaming-classifier.txt"
        else if classifier.preset == "social-media" then
          "${packages.classifier}/share/dns-smart-block/prompts/social-media-classifier.txt"
        else
          classifier.customTemplate
      ) enabledClassifiers
    );

  in {
    # Install packages.
    environment.systemPackages = builtins.attrValues packages ++ [ pkgs.natscli ];

    # Create service user and group for PostgreSQL peer authentication.
    users.users.${serviceUser} = {
      isSystemUser = true;
      group = serviceGroup;
      description = "DNS Smart Block service user";
    };

    users.groups.${serviceGroup} = {};

    # NATS Service.  Only the minimal JetStream requirement is set here; all
    # tuning (storage limits, monitoring ports, etc.) is left to the operator
    # via services.nats.
    services.nats = mkIf cfg.nats.enable {
      enable = true;
      jetstream = true;
    };

    # PostgreSQL Service.
    services.postgresql = mkIf cfg.database.enable {
      enable = true;
      ensureDatabases = [ cfg.database.name ];
      ensureUsers = [{
        name = cfg.database.user;
        ensureDBOwnership = true;
      }];
    };

    # Queue Processor Service (single unified service).
    systemd.services = {
      # Single queue processor that runs all classifiers.
      dns-smart-block-queue-processor = mkIf ((lib.length (lib.attrNames enabledClassifiers)) > 0) {
        description = "DNS Smart Block Queue Processor";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ]
                ++ lib.optional cfg.nats.enable "dns-smart-block-nats-init.service"
                ++ lib.optional cfg.database.enable "postgresql.service";
        wants = lib.optional cfg.nats.enable "dns-smart-block-nats-init.service"
                ++ lib.optional cfg.database.enable "postgresql.service";
        requires = lib.optional cfg.database.enable "postgresql.service";

        serviceConfig = {
          Type = "simple";
          User = serviceUser;
          Group = serviceGroup;

          ExecStart = let
            args = lib.concatStringsSep " " ([
              "${packages.queue-processor}/bin/dns-smart-block-queue-processor"
              "--nats-url '${cfg.nats.url}'"
              "--nats-subject '${cfg.nats.subject}'"
              "--nats-max-ack-pending ${toString cfg.nats.maxAckPending}"
              "--database-url '${databaseUrl}'"
              "--classifier-path '${packages.classifier}/bin/dns-smart-block-classifier'"
              "--config-file '${queueProcessorTomlConfig}'"
            ] ++ lib.optionals (cfg.database.passwordFile != null) [
              "--database-password-file '${cfg.database.passwordFile}'"
            ]);
          in args;

          Restart = "always";
          RestartSec = "5s";

          # Security hardening.
          NoNewPrivileges = true;
          PrivateTmp = true;
          ProtectSystem = "strict";
          ProtectHome = true;

          # Read access to config file and all prompt templates.
          ReadOnlyPaths = [ queueProcessorTomlConfig ] ++ allPromptTemplates;
        };

        environment = {
          RUST_LOG = "info";
        };
      };

      # Log Processor Service.
      dns-smart-block-log-processor = mkIf cfg.logProcessor.enable {
        description = "DNS Smart Block Log Processor";
        wantedBy = [ "multi-user.target" ];
        after =
          [ "network.target" ]
          ++ lib.optional cfg.nats.enable "dns-smart-block-nats-init.service"
          ++ lib.optional
            (lib.hasPrefix "cmd:journalctl" cfg.logProcessor.logSource)
            "systemd-journald.service"
        ;
        wants = lib.optional cfg.nats.enable "dns-smart-block-nats-init.service";

        serviceConfig = {
          Type = "simple";
          User = serviceUser;
          Group = serviceGroup;

          # Grant access to systemd journal if using journalctl.
          SupplementaryGroups = lib.optional
            (lib.hasPrefix "cmd:journalctl" cfg.logProcessor.logSource)
            "systemd-journal";

          ExecStart = let
            args = lib.concatStringsSep " " [
              "${packages.log-processor}/bin/dns-smart-block-log-processor"
              "--log-source '${cfg.logProcessor.logSource}'"
              "--nats-url '${cfg.nats.url}'"
              "--nats-subject '${cfg.nats.subject}'"
            ];
          in args;

          Restart = "always";
          RestartSec = "5s";

          # Security hardening.
          NoNewPrivileges = true;
          PrivateTmp = true;
          ProtectSystem = "strict";
          ProtectHome = true;

          # Read-only access to logs if using file source.
          ReadOnlyPaths = lib.optional
            (!(lib.hasPrefix "cmd:" cfg.logProcessor.logSource))
            cfg.logProcessor.logSource
          ;
        };

        environment = {
          RUST_LOG = "info";
        };
      };

      # Idempotently create the DNS_SMART_BLOCK JetStream stream after NATS
      # starts.  The || true makes the command succeed even if the stream
      # already exists, so this is safe to run on every activation.
      dns-smart-block-nats-init = mkIf cfg.nats.enable {
        description = "Initialize DNS Smart Block NATS stream";
        wantedBy = [ "multi-user.target" ];
        after = [ "nats.service" ];
        requires = [ "nats.service" ];
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          ExecStart = pkgs.writeShellScript "dns-smart-block-nats-init" ''
            # Poll until NATS accepts connections before creating the stream.
            until ${pkgs.natscli}/bin/nats --server=${cfg.nats.url} server ping >/dev/null 2>&1; do
              sleep 1
            done
            ${pkgs.natscli}/bin/nats --server=${cfg.nats.url} stream add DNS_SMART_BLOCK \
              --subjects="${cfg.nats.subject}" \
              --storage=file \
              --retention=limits \
              --max-age=7d \
              --dupe-window=2m \
              --replicas=1 \
              --defaults || true
          '';
          NoNewPrivileges = true;
          PrivateTmp = true;
          ProtectSystem = "strict";
          ProtectHome = true;
        };
      };
      # Blocklist Server Service.
      dns-smart-block-blocklist-server = mkIf cfg.blocklistServer.enable {
        description = "DNS Smart Block Blocklist Server";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ]
                ++ lib.optional cfg.database.enable "postgresql.service"
                ++ lib.optional cfg.nats.enable "dns-smart-block-nats-init.service";
        wants = lib.optional cfg.database.enable "postgresql.service"
                ++ lib.optional cfg.nats.enable "dns-smart-block-nats-init.service";
        requires = lib.optional cfg.database.enable "postgresql.service";

        serviceConfig = {
          Type = "simple";
          User = serviceUser;
          Group = serviceGroup;

          ExecStart = let
            publicBindAddress = "${cfg.blocklistServer.publicBindHost}:${toString cfg.blocklistServer.publicBindPort}";
            adminBindAddress = "${cfg.blocklistServer.adminBindHost}:${toString cfg.blocklistServer.adminBindPort}";
            args = lib.concatStringsSep " " ([
              "${packages.blocklist-server}/bin/dns-smart-block-blocklist-server"
              "--database-url '${databaseUrl}'"
              "--public-bind-address '${publicBindAddress}'"
              "--admin-bind-address '${adminBindAddress}'"
              "--nats-url '${cfg.nats.url}'"
              "--nats-subject '${cfg.nats.subject}'"
            ] ++ lib.optionals (cfg.database.passwordFile != null) [
              "--database-password-file '${cfg.database.passwordFile}'"
            ]);
          in args;

          Restart = "always";
          RestartSec = "5s";
          # Security hardening.
          NoNewPrivileges = true;
          PrivateTmp = true;
          ProtectSystem = "strict";
          ProtectHome = true;
        };

        environment = {
          RUST_LOG = "info";
        };
      };
    };

    # Assertions and warnings.
    assertions = [
      {
        assertion = ((
          lib.length
            (lib.attrNames enabledClassifiers)) > 0
        )
          -> (cfg.nats.enable || cfg.nats.url != "")
        ;
        message = ''
          NATS must be enabled or URL configured when classifiers are enabled
        '';
      }
      {
        assertion =
          cfg.logProcessor.enable -> (cfg.nats.enable || cfg.nats.url != "");
        message = ''
          NATS must be enabled or URL configured when log processor is enabled
        '';
      }
      {
        assertion =
          cfg.queueProcessor.minConfidence >= 0.0
          && cfg.queueProcessor.minConfidence <= 1.0
        ;
        message = ''
          services.dns-smart-block.queueProcessor.minConfidence must be between 0.0 and 1.0
        '';
      }
    ] ++ (lib.concatLists (lib.mapAttrsToList (name: classifier: [
      {
        assertion =
          classifier.enable ->
          (classifier.preset != null || classifier.customTemplate != null)
        ;
        message = ''
          services.dns-smart-block.classifiers.${name}: Either preset or customTemplate must be set
        '';
      }
      {
        assertion =
          classifier.minConfidence >= 0.0
          && classifier.minConfidence <= 1.0;
        message = ''
          services.dns-smart-block.classifiers.${name}.minConfidence must be between 0.0 and 1.0
        '';
      }
    ]) cfg.classifiers));

    warnings =
      lib.optional
        (!cfg.nats.enable
         && cfg.nats.url == "nats://localhost:4222")
        ''
          DNS Smart Block: Built-in NATS is disabled but no external NATS URL configured
        ''
      ;
  });
}
