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
      example = "http://silicon.proton:3000";
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
    # Auto-mapping: map all locally enabled classifiers.
    services.blocky.settings.blocking.blackLists = mkIf blockyIntegrationCfg.autoMapAllBlocklists (
      lib.mapAttrs
        (classifierName: _classifier: [
          "${blockyIntegrationCfg.blocklistUrl}/blocklist?type=${classifierName}"
        ])
        enabledClassifiers
    );

    # Manual mapping: use provided mappings.
    services.blocky.settings.blocking.blackLists = mkIf (blockyIntegrationCfg.blocklistMappings != null) (
      lib.mapAttrs
        (groupName: classifierType: [
          "${blockyIntegrationCfg.blocklistUrl}/blocklist?type=${classifierType}"
        ])
        blockyIntegrationCfg.blocklistMappings
    );

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
