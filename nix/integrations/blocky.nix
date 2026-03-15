{ config, lib, ... }: let
  cfg = config.services.dns-smart-block;
  inherit (lib) mkEnableOption mkOption mkIf types;

  blockyIntegrationCfg = cfg.integrations.blocky;

  # Get list of enabled classifiers.
  enabledClassifiers = lib.filterAttrs (name: classifier: classifier.enable) cfg.classifiers;

in {
  options.services.dns-smart-block.integrations.blocky = {
    enable = mkEnableOption "Blocky DNS server integration";

    blocklistUrl = mkOption {
      type = types.str;
      default = "http://localhost:3000";
      example = "http://dns-smart-block.example.com:3000";
      description = ''
        Base URL of the dns-smart-block blocklist server.
        The integration will append /blocklist?type=<classifier> to this URL.
      '';
    };

    autoMapAllBlocklists = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Automatically map all locally enabled classifiers to Blocky blacklist groups.
        Each classifier will be mapped to a Blocky group with the same name.

        This is a LOCAL-only mapping: it maps classifiers that are enabled on the
        same host where this option is set.

        Mutually exclusive with blocklistMappings.
      '';
    };

    blocklistMappings = mkOption {
      type = types.nullOr (types.attrsOf types.str);
      default = null;
      example = {
        gaming = "gaming";
        video-streaming = "video-streaming";
        custom-category = "my-custom-classifier";
      };
      description = ''
        Manual mapping of Blocky blacklist group names to classifier types.
        The attribute names are the Blocky group names, and the values are the
        classifier type names to fetch from the blocklist server.

        This allows cross-host configurations where Blocky on one host points
        to a blocklist server on another host.

        Mutually exclusive with autoMapAllBlocklists.
      '';
    };
  };

  config = mkIf blockyIntegrationCfg.enable {
    # Wire the log processor to consume Blocky's journald output and parse only
    # successfully resolved external queries.
    services.dns-smart-block.logProcessor = {
      logSource = lib.mkDefault
        "cmd:journalctl --follow --unit=blocky.service";

      # Extracts the domain from Blocky's structured log field, stripping the
      # trailing FQDN dot that Blocky always appends.
      domainPattern = lib.mkDefault
        ''question_name=(\w(?:[\w-]*\w)?(?:\.\w(?:[\w-]*\w)?)+)\.'';

      # Only process lines where Blocky actually forwarded the query to an
      # upstream resolver.  This excludes blocked (response_type=BLOCKED),
      # cached (response_type=CACHED), local/conditional
      # (response_type=CONDITIONAL), and NXDOMAIN entries — all of which would
      # produce either garbage or already-blocked domains.
      lineFilter = lib.mkDefault "response_type=RESOLVED";
    };

    # Configure blocky blacklists based on either auto-mapping or manual mapping.
    services.blocky.settings.blocking.blackLists =
      if blockyIntegrationCfg.autoMapAllBlocklists then
        # Auto-mapping: map all locally enabled classifiers.
        lib.mapAttrs
          (classifierName: _classifier: [
            "${blockyIntegrationCfg.blocklistUrl}/blocklist?type=${classifierName}"
          ])
          enabledClassifiers
      else if blockyIntegrationCfg.blocklistMappings != null then
        # Manual mapping: use provided mappings.
        lib.mapAttrs
          (groupName: classifierType: [
            "${blockyIntegrationCfg.blocklistUrl}/blocklist?type=${classifierType}"
          ])
          blockyIntegrationCfg.blocklistMappings
      else
        {};

    # Assertion: prevent both auto and manual mapping.
    assertions = [
      {
        assertion =
          !(blockyIntegrationCfg.autoMapAllBlocklists && blockyIntegrationCfg.blocklistMappings != null);
        message = ''
          services.dns-smart-block.integrations.blocky: autoMapAllBlocklists and blocklistMappings are mutually exclusive.
          Please enable only one of them.
        '';
      }
    ];
  };
}
