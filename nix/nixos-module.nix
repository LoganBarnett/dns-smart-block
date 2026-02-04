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
        type = types.nullOr (types.enum [ "gaming" "video-streaming" ]);
        default = null;
        example = "gaming";
        description = ''
          Use a bundled classifier preset. Available presets:
          - "gaming" - Classifies gaming-related websites
          - "video-streaming" - Classifies video streaming platforms

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
        default = 7;
        description = "Time-to-live in days for cached classifications. After this many days, a domain will be re-classified.";
      };
    };
  };

in {
  imports = [
    ./integrations/blocky.nix
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
        default = 10;
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
    };

    # Blocklist Server Configuration
    blocklistServer = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Enable HTTP blocklist server for serving DNS blocklists";
      };

      bindAddress = mkOption {
        type = types.str;
        default = "127.0.0.1:3000";
        description = "Address and port to bind the blocklist server to";
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

    # Generate a queue processor service for a classifier.
    mkQueueProcessorService = classifierName: classifier: let
      # Determine which prompt template to use.
      promptTemplate =
        if classifier.preset == "gaming" then
          "${packages.classifier}/share/dns-smart-block/prompts/gaming-classifier.txt"
        else if classifier.preset == "video-streaming" then
          "${packages.classifier}/share/dns-smart-block/prompts/video-streaming-classifier.txt"
        else
          classifier.customTemplate;
    in {
      description = "DNS Smart Block Queue Processor - ${classifierName}";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ]
              ++ lib.optional cfg.nats.enable "dns-smart-block-nats.service"
              ++ lib.optional cfg.database.enable "postgresql.service";
      wants = lib.optional cfg.nats.enable "dns-smart-block-nats.service"
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
            "--ollama-url '${cfg.ollama.url}'"
            "--ollama-model '${cfg.ollama.model}'"
            "--prompt-template '${promptTemplate}'"
            "--classification-type '${classifierName}'"
            "--http-timeout-sec ${toString classifier.httpTimeoutSec}"
            "--http-max-kb ${toString classifier.httpMaxKb}"
            "--min-confidence ${toString classifier.minConfidence}"
            "--ttl-days ${toString classifier.ttlDays}"
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

        # Read access to prompt template.
        ReadOnlyPaths = [ promptTemplate ];
      };

      environment = {
        RUST_LOG = "info";
      };
    };

  in {
    # Install packages.
    environment.systemPackages = builtins.attrValues packages;

    # Create service user and group for PostgreSQL peer authentication.
    users.users.${serviceUser} = {
      isSystemUser = true;
      group = serviceGroup;
      description = "DNS Smart Block service user";
    };

    users.groups.${serviceGroup} = {};

    # PostgreSQL Service.
    services.postgresql = mkIf cfg.database.enable {
      enable = true;
      ensureDatabases = [ cfg.database.name ];
      ensureUsers = [{
        name = cfg.database.user;
        ensureDBOwnership = true;
      }];
    };

    # Queue Processor Services (one per enabled classifier) - merged into
    # systemd.services below.
    systemd.services = (
      lib.mapAttrs'
        (name: classifier: lib.nameValuePair "dns-smart-block-queue-processor-${name}" (mkQueueProcessorService name classifier))
        enabledClassifiers
    ) // {

      # Log Processor Service.
      dns-smart-block-log-processor = mkIf cfg.logProcessor.enable {
        description = "DNS Smart Block Log Processor";
        wantedBy = [ "multi-user.target" ];
        after =
          [ "network.target" ]
          ++ lib.optional cfg.nats.enable "dns-smart-block-nats.service"
          ++ lib.optional cfg.database.enable "postgresql.service"
          ++ lib.optional
            (lib.hasPrefix "cmd:journalctl" cfg.logProcessor.logSource)
            "systemd-journald.service"
        ;
        wants = lib.optional cfg.nats.enable "dns-smart-block-nats.service"
                ++ lib.optional cfg.database.enable "postgresql.service";
        requires = lib.optional cfg.database.enable "postgresql.service";

        serviceConfig = {
          Type = "simple";
          User = serviceUser;
          Group = serviceGroup;

          # Grant access to systemd journal if using journalctl.
          SupplementaryGroups = lib.optional
            (lib.hasPrefix "cmd:journalctl" cfg.logProcessor.logSource)
            "systemd-journal";

          ExecStart = let
            args = lib.concatStringsSep " " ([
              "${packages.log-processor}/bin/dns-smart-block-log-processor"
              "--log-source '${cfg.logProcessor.logSource}'"
              "--nats-url '${cfg.nats.url}'"
              "--nats-subject '${cfg.nats.subject}'"
              "--database-url '${databaseUrl}'"
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

      # NATS Server Service.
      dns-smart-block-nats = mkIf cfg.nats.enable (let
        # Generate NATS configuration file for JetStream persistence.
        natsConfig = pkgs.writeText "nats-server.conf" ''
          # NATS Server Configuration for DNS Smart Block
          port: ${toString cfg.nats.port}

          # HTTP monitoring endpoint for metrics and stats.
          http: 0.0.0.0:8222

          # Enable JetStream for message persistence.
          jetstream {
            # Store JetStream data in the configured directory.
            store_dir: "${cfg.nats.dataDir}"

            # Maximum storage size for JetStream (1GB).
            max_memory_store: 1GB
            max_file_store: 1GB
          }

          # Define a stream for domain classification messages.
          # This stream automatically persists all messages published to the subject,
          # allowing clients to use basic pub/sub while getting durability.
          #
          # Note: Stream creation via config file requires NATS 2.10+.
          # For older versions, streams must be created via nats CLI or API.
        '';
      in {
        description = "NATS server for DNS Smart Block";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];

        serviceConfig = {
          Type = "simple";
          DynamicUser = true;
          StateDirectory = "dns-smart-block-nats";
          # Use configuration file to enable JetStream and configure streams.
          ExecStart = ''
            ${pkgs.nats-server}/bin/nats-server \
               -c ${natsConfig}
          '';
          Restart = "always";
          RestartSec = "5s";
          # Security hardening.
          NoNewPrivileges = true;
          PrivateTmp = true;
          ProtectSystem = "strict";
          ProtectHome = true;
          ReadWritePaths = [ cfg.nats.dataDir ];
        };
        # Stream must be created after server starts since config file
        # stream definitions aren't supported in all NATS versions.
        postStart = ''
          # Wait for NATS server to be ready.
          sleep 2

          # Create JetStream stream for domain messages using nats CLI.
          # This stream will persist all messages on the subject.
          # --defaults flag accepts default values for all prompts (needed for non-interactive context).
          ${pkgs.natscli}/bin/nats --server=nats://localhost:${toString cfg.nats.port} stream add DNS_SMART_BLOCK \
            --subjects="${cfg.nats.subject}" \
            --storage=file \
            --retention=limits \
            --max-msgs=-1 \
            --max-bytes=1GB \
            --max-age=7d \
            --max-msg-size=1MB \
            --discard=old \
            --dupe-window=2m \
            --replicas=1 \
            --defaults || true
        '';
      });
      # Blocklist Server Service.
      dns-smart-block-blocklist-server = mkIf cfg.blocklistServer.enable {
        description = "DNS Smart Block Blocklist Server";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ]
                ++ lib.optional cfg.database.enable "postgresql.service";
        wants = lib.optional cfg.database.enable "postgresql.service";
        requires = lib.optional cfg.database.enable "postgresql.service";

        serviceConfig = {
          Type = "simple";
          User = serviceUser;
          Group = serviceGroup;

          ExecStart = let
            args = lib.concatStringsSep " " ([
              "${packages.blocklist-server}/bin/dns-smart-block-blocklist-server"
              "--database-url '${databaseUrl}'"
              "--bind-address '${cfg.blocklistServer.bindAddress}'"
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
         && cfg.nats.url == "nats://localhost:${toString cfg.nats.port}")
        ''
          DNS Smart Block: Built-in NATS is disabled but no external NATS URL configured
        ''
      ;
  });
}
